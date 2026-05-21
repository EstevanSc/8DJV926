use crate::authority::AuthorityState;

/// Verifies the default authority state is Owned.
#[test]
fn default_state_is_owned() {
    assert_eq!(AuthorityState::default(), AuthorityState::Owned);
}

/// Verifies Owned and PendingHandoff allow local simulation.
#[test]
fn owned_and_pending_allow_local_simulation() {
    assert!(AuthorityState::Owned.allows_local_simulation());
    assert!(AuthorityState::PendingHandoff.allows_local_simulation());
}

/// Verifies Ghost blocks local simulation and snapshots.
#[test]
fn ghost_disables_local_simulation_and_snapshots() {
    assert!(!AuthorityState::Ghost.allows_local_simulation());
    assert!(!AuthorityState::Ghost.is_snapshot_visible());
    assert!(AuthorityState::Ghost.is_ghost());
}