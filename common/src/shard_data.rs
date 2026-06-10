//! Shared shard data structures for quadtree and orchestrator communication.

use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};
use std::hash::{Hash, Hasher};

/// Boundary of a shard in 2D space (axis-aligned bounding box).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, SchemaWrite, SchemaRead)]
pub struct Boundary {
    pub x: f64,
    pub y: f64,
    pub half_size: f64,
}

impl PartialEq for Boundary {
    fn eq(&self, other: &Self) -> bool {
        self.x.to_bits() == other.x.to_bits()
            && self.y.to_bits() == other.y.to_bits()
            && self.half_size.to_bits() == other.half_size.to_bits()
    }
}

impl Eq for Boundary {}

impl Hash for Boundary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.x.to_bits().hash(state);
        self.y.to_bits().hash(state);
        self.half_size.to_bits().hash(state);
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Quadrant {
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

impl Boundary {
    /// Check if a point is contained within this boundary.
    pub fn contains(&self, x: &f64, y: &f64) -> bool {
        let left = self.x - self.half_size;
        let right = self.x + self.half_size;
        let top = self.y - self.half_size;
        let bottom = self.y + self.half_size;

        *x >= left && *x < right &&
        *y >= top  && *y < bottom
    }

    /// Determine which quadrant a point falls into.
    pub fn quadrant(&self, x: &f64, y: &f64) -> Quadrant {
        if *x >= self.x {
            if *y < self.y {
                Quadrant::NorthEast
            } else {
                Quadrant::SouthEast
            }
        } else {
            if *y < self.y {
                Quadrant::NorthWest
            } else {
                Quadrant::SouthWest
            }
        }
    }

    /// Subdivide this boundary into 4 quadrants.
    pub fn subdivide(&self) -> [Boundary; 4] {
        let hs = self.half_size / 2.0;
        [
            Boundary { x: self.x + hs, y: self.y - hs, half_size: hs }, // NE
            Boundary { x: self.x - hs, y: self.y - hs, half_size: hs }, // NW
            Boundary { x: self.x + hs, y: self.y + hs, half_size: hs }, // SE
            Boundary { x: self.x - hs, y: self.y + hs, half_size: hs }, // SW
        ]
    }

    /// Check if this boundary intersects with a range around a point.
    pub fn intersects_range(&self, x: &f64, y: &f64, margin: f64) -> bool {
        let self_left = self.x - self.half_size;
        let self_right = self.x + self.half_size;
        let self_top = self.y - self.half_size;
        let self_bottom = self.y + self.half_size;

        let range_left = *x - margin;
        let range_right = *x + margin;
        let range_top = *y - margin;
        let range_bottom = *y + margin;

        // Returns true if the two AABBs overlap
        self_left < range_right && self_right > range_left &&
        self_top < range_bottom && self_bottom > range_top
    }

    pub fn encode_batch(boundaries: &Vec<Boundary>) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(boundaries)
    }

    pub fn decode_batch(data: &[u8]) -> serde_json::Result<Vec<Boundary>> {
        serde_json::from_slice(data)
    }
}
