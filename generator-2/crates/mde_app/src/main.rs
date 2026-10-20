use bevy::prelude::*;
use bevy_egui::EguiPlugin;

fn main() {
    App::new().add_plugins((DefaultPlugins, EguiPlugin::default())).run();
}
