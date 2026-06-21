use bevy_behave::ego_tree::Tree;
use bevy_behave::prelude::*;
use uuid::Uuid;

use super::actions::chase::{Chase, CheckNearby};
use super::actions::fireball::{CastFireball, CheckAggroDistance};
use super::actions::heal::{CastHeal, CheckLowHealth};
use super::actions::patrol::Patrol;

/// Builds the behaviour tree for a Goblin:
/// - Fallback: try to chase if a player is nearby, otherwise patrol
pub fn goblin_tree(_id: Uuid) -> BehaveTree {
    let tree: Tree<Behave> = behave! {
        Behave::Forever => {
            Behave::Fallback => {
                Behave::Sequence => {
                    Behave::trigger(CheckAggroDistance),
                    Behave::trigger(CastFireball),
                },
                Behave::Fallback => {
                    Behave::Sequence => { Behave::trigger(CheckNearby), Behave::trigger(Chase) },
                    Behave::Sequence => {
                        Behave::trigger(CheckLowHealth),
                        Behave::trigger(CastHeal),
                    },
                    Behave::trigger(Patrol),
                }
            }
        }
    };
    BehaveTree::new(tree)
}

/// Dispatches to the correct tree based on AI kind.
pub fn build_tree(id: Uuid) -> BehaveTree {
    goblin_tree(id)
}
