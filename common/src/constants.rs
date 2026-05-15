/// Minimum movement (in world units) required before a position update is sent.
/// Updates with delta below this value are skipped to save bandwidth.
pub const POSITION_DELTA_THRESHOLD: f32 = 0.1;
