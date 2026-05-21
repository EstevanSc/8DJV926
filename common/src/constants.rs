/// Minimum movement (in world units) required before a position update is sent.
/// Updates with delta below this value are skipped to save bandwidth.
pub const POSITION_DELTA_THRESHOLD: f32 = 0.1;

/// Spatial interest radius in tiles — only entities within this range receive
/// position snapshots for a given player.
pub const INTEREST_RADIUS_TILES: u32 = 32;
