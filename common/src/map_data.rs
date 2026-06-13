pub const MAP_WIDTH: usize = 1 * 64;
pub const MAP_HEIGHT: usize = 1 * 64;
pub const TILE_SIZE: f32 = 32.0;

pub struct BitMap {
    // 64 rows, each holding 1 u64 integer (1 * 64 = 64 bits per row)
    pub data: [[u64; 1]; MAP_HEIGHT],
}

impl BitMap {
    /// Creates an empty map (all 0s / empty space)
    pub fn new() -> Self {
        Self {
            data: [[0; 1]; MAP_HEIGHT],
        }
    }

    /// Sets a wall (1) at (x, y)
    pub fn set_wall(&mut self, x: usize, y: usize) {
        if x < MAP_WIDTH && y < MAP_HEIGHT {
            let bucket = x / 64;
            let bit = x % 64;
            self.data[y][bucket] |= 1 << bit;
        }
    }

    /// Returns true if there is a wall (1) at (x, y)
    pub fn is_wall(&self, x: usize, y: usize) -> bool {
        if x >= MAP_WIDTH || y >= MAP_HEIGHT {
            return true; // Out of bounds acts as a wall
        }
        let bucket = x / 64;
        let bit = x % 64;
        (self.data[y][bucket] & (1 << bit)) != 0
    }

    /// Fills the map with a playable layout
    pub fn generate_map(&mut self) {
        for y in 0..MAP_HEIGHT {
            for x in 0..MAP_WIDTH {
                // 1. Create Outer Border Walls
                if x == 0 || x == MAP_WIDTH - 1 || y == 0 || y == MAP_HEIGHT - 1 {
                    self.set_wall(x, y);
                }
                // 2. Create Internal Obstacles (Pillars every 8 tiles)
                if x % 8 == 0 && y % 8 == 0 {
                    self.set_wall(x, y);
                }
            }
        }
    }

    /// Prints a specific window of the map to the console for inspection
    pub fn print_sub_grid(&self, start_x: usize, start_y: usize, width: usize, height: usize) {
        for y in start_y..(start_y + height) {
            for x in start_x..(start_x + width) {
                if self.is_wall(x, y) {
                    print!("#"); // Wall
                } else {
                    print!("."); // Empty playable space
                }
            }
            println!();
        }
    }
}