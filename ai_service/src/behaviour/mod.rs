pub mod actions;
pub mod trees;

use bevy::prelude::*;
use bevy_behave::prelude::*;

use actions::{
    chase::{on_chase, on_check_nearby},
    fireball::{on_cast_fireball, on_check_aggro_distance},
    heal::{on_cast_heal, on_check_low_health},
    patrol::on_patrol,
};

/// Bevy plugin that registers the behaviour tree plugin and all action observers.
pub struct BehaviourPlugin;

impl Plugin for BehaviourPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(BehavePlugin::default())
            .add_observer(on_patrol)
            .add_observer(on_chase)
            .add_observer(on_check_nearby)
            .add_observer(on_check_low_health)
            .add_observer(on_cast_heal)
            .add_observer(on_check_aggro_distance)
            .add_observer(on_cast_fireball);
    }
}
