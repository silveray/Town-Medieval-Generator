//! Binary glTF 2.0 (`.glb`) export.
//!
//! One node and one mesh per [`Model`], one primitive per [`mde_core::MeshPart`],
//! one material per [`MaterialSlot`] used (named after the slot, PBR base colour
//! and roughness from [`crate::palette`], metallic 0). Vertex attributes:
//! `POSITION` (with min / max), `NORMAL`, `TEXCOORD_0` and `COLOR_0` (float
//! `VEC4`, linear) when present; `u32` indices. Buffer views are 4-byte aligned,
//! the JSON chunk is padded with spaces and the BIN chunk with zeros.

use mde_core::{MaterialSlot, MeshData, Model};
use serde_json::{json, Map, Value};

use crate::palette::{slot_color, slot_name, slot_roughness};

const GLB_MAGIC: u32 = 0x4654_6C67; // "glTF"
const CHUNK_JSON: u32 = 0x4E4F_534A; // "JSON"
const CHUNK_BIN: u32 = 0x004E_4942; // "BIN\0"
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

/// Encode `models` as a binary glTF 2.0 file (`asset.generator` =
/// `"midearth mde_mesh"`). Empty parts are skipped; a model without geometry
/// becomes a node without a mesh. Non-finite attribute values are written as 0.
pub fn write_glb(models: &[Model]) -> Vec<u8> {
    let (json, bin) = build(models);
    let mut json_bytes = serde_json::to_vec(&json).unwrap_or_else(|_| b"{}".to_vec());
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin = bin;
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let has_bin = !bin.is_empty();
    let total = 12 + 8 + json_bytes.len() + if has_bin { 8 + bin.len() } else { 0 };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&GLB_MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json_bytes);
    if has_bin {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    out
}

/// Split a `.glb` into its JSON document and BIN chunk (for tests and tools).
/// Returns `None` if the container is malformed.
pub fn read_glb(data: &[u8]) -> Option<(Value, Vec<u8>)> {
    let u32_at = |o: usize| -> Option<u32> { Some(u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?)) };
    if u32_at(0)? != GLB_MAGIC || u32_at(4)? != 2 || u32_at(8)? as usize != data.len() {
        return None;
    }
    let json_len = u32_at(12)? as usize;
    if u32_at(16)? != CHUNK_JSON {
        return None;
    }
    let json: Value = serde_json::from_slice(data.get(20..20 + json_len)?).ok()?;
    let mut bin = Vec::new();
    let o = 20 + json_len;
    if o < data.len() {
        let bin_len = u32_at(o)? as usize;
        if u32_at(o + 4)? != CHUNK_BIN {
            return None;
        }
        bin = data.get(o + 8..o + 8 + bin_len)?.to_vec();
    }
    Some((json, bin))
}

struct Writer {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Writer {
    fn view(&mut self, bytes: &[u8], target: u32) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let offset = self.bin.len();
        self.bin.extend_from_slice(bytes);
        self.views.push(json!({
            "buffer": 0,
            "byteOffset": offset,
            "byteLength": bytes.len(),
            "target": target,
        }));
        self.views.len() - 1
    }

    fn float_accessor<const N: usize>(&mut self, data: &[[f32; N]], kind: &str, min_max: bool) -> usize {
        let mut bytes = Vec::with_capacity(data.len() * N * 4);
        let mut min = [f32::INFINITY; N];
        let mut max = [f32::NEG_INFINITY; N];
        for v in data {
            for (k, &x) in v.iter().enumerate() {
                let x = if x.is_finite() { x } else { 0.0 };
                bytes.extend_from_slice(&x.to_le_bytes());
                min[k] = min[k].min(x);
                max[k] = max[k].max(x);
            }
        }
        let view = self.view(&bytes, ARRAY_BUFFER);
        let mut acc = json!({
            "bufferView": view,
            "componentType": FLOAT,
            "count": data.len(),
            "type": kind,
        });
        if min_max && !data.is_empty() {
            acc["min"] = json!(min.to_vec());
            acc["max"] = json!(max.to_vec());
        }
        self.accessors.push(acc);
        self.accessors.len() - 1
    }

    fn index_accessor(&mut self, idx: &[u32]) -> usize {
        let mut bytes = Vec::with_capacity(idx.len() * 4);
        let (mut min, mut max) = (u32::MAX, 0u32);
        for &i in idx {
            bytes.extend_from_slice(&i.to_le_bytes());
            min = min.min(i);
            max = max.max(i);
        }
        let view = self.view(&bytes, ELEMENT_ARRAY_BUFFER);
        self.accessors.push(json!({
            "bufferView": view,
            "componentType": UNSIGNED_INT,
            "count": idx.len(),
            "type": "SCALAR",
            "min": [min],
            "max": [max],
        }));
        self.accessors.len() - 1
    }
}

/// Whether a mesh can be written as a valid primitive.
fn usable(m: &MeshData) -> bool {
    let n = m.positions.len();
    !m.indices.is_empty()
        && m.indices.len().is_multiple_of(3)
        && m.indices.iter().all(|&i| (i as usize) < n)
        && (m.normals.is_empty() || m.normals.len() == n)
        && (m.uvs.is_empty() || m.uvs.len() == n)
        && (m.colors.is_empty() || m.colors.len() == n)
}

fn build(models: &[Model]) -> (Value, Vec<u8>) {
    let mut w = Writer { bin: Vec::new(), views: Vec::new(), accessors: Vec::new() };
    let mut materials: Vec<MaterialSlot> = Vec::new();
    let mut nodes = Vec::new();
    let mut meshes = Vec::new();
    for model in models {
        let mut prims = Vec::new();
        for part in &model.parts {
            let m = &part.mesh;
            if !usable(m) {
                continue;
            }
            let mat = match materials.iter().position(|&s| s == part.material) {
                Some(i) => i,
                None => {
                    materials.push(part.material);
                    materials.len() - 1
                }
            };
            let mut attrs = Map::new();
            attrs.insert("POSITION".into(), json!(w.float_accessor(&m.positions, "VEC3", true)));
            if !m.normals.is_empty() {
                let normals: Vec<[f32; 3]> = m
                    .normals
                    .iter()
                    .map(|n| {
                        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                        if l.is_finite() && l > 1e-12 { [n[0] / l, n[1] / l, n[2] / l] } else { [0.0, 1.0, 0.0] }
                    })
                    .collect();
                attrs.insert("NORMAL".into(), json!(w.float_accessor(&normals, "VEC3", false)));
            }
            if !m.uvs.is_empty() {
                attrs.insert("TEXCOORD_0".into(), json!(w.float_accessor(&m.uvs, "VEC2", false)));
            }
            if !m.colors.is_empty() {
                attrs.insert("COLOR_0".into(), json!(w.float_accessor(&m.colors, "VEC4", false)));
            }
            let indices = w.index_accessor(&m.indices);
            prims.push(json!({
                "attributes": Value::Object(attrs),
                "indices": indices,
                "material": mat,
                "mode": 4,
            }));
        }
        let mut node = json!({ "name": model.name });
        if !prims.is_empty() {
            node["mesh"] = json!(meshes.len());
            meshes.push(json!({ "name": model.name, "primitives": prims }));
        }
        nodes.push(node);
    }
    let mats: Vec<Value> = materials
        .iter()
        .map(|&s| {
            json!({
                "name": slot_name(s),
                "pbrMetallicRoughness": {
                    "baseColorFactor": slot_color(s).to_vec(),
                    "metallicFactor": 0.0,
                    "roughnessFactor": slot_roughness(s),
                },
            })
        })
        .collect();
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": "midearth mde_mesh" },
        "scene": 0,
        "scenes": [{ "name": "midearth", "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": nodes,
    });
    if !meshes.is_empty() {
        doc["meshes"] = json!(meshes);
        doc["materials"] = json!(mats);
        doc["accessors"] = json!(w.accessors);
        doc["bufferViews"] = json!(w.views);
        let mut len = w.bin.len();
        while !len.is_multiple_of(4) {
            len += 1;
        }
        doc["buffers"] = json!([{ "byteLength": len }]);
    }
    (doc, w.bin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mde_core::MeshPart;

    #[test]
    fn empty_and_minimal() {
        let glb = write_glb(&[]);
        let (json, bin) = read_glb(&glb).unwrap();
        assert!(bin.is_empty());
        assert_eq!(json["asset"]["generator"], "midearth mde_mesh");

        let mesh = MeshData {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        let model = Model { name: "t".into(), parts: vec![MeshPart { material: MaterialSlot::Stone, mesh }] };
        let glb = write_glb(&[model, Model::new("empty")]);
        assert_eq!(glb.len() % 4, 0);
        let (json, bin) = read_glb(&glb).unwrap();
        assert_eq!(json["nodes"].as_array().unwrap().len(), 2);
        assert!(json["nodes"][1].get("mesh").is_none());
        assert_eq!(json["materials"][0]["name"], "Stone");
        assert_eq!(json["buffers"][0]["byteLength"].as_u64().unwrap() as usize, bin.len());
    }
}
