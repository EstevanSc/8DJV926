use std::mem;

const WORLD_SIZE: f64 = 100.0;
const MAX_CAPACITY: usize = 4;
const MAX_DEPTH: u8 = 10;
const NEARBY_MARGIN: f64 = 5.0;

fn main() {
    let boundary = Boundary {
        x: 0.0,
        y: 0.0,
        half_size: WORLD_SIZE / 2.0,
    };

    let mut quadtree = Quadtree::new(boundary, 0);

    let mut entities = std::collections::HashMap::new();

    let mut shards = std::collections::HashSet::new();

    let mut counter = 0;

    //simulate a publish-subscribe system where entities are added to the quadtree and we query for nearby shards
    loop {
        // Simulate entity creation

        let id: u32;        
        let pos: Vec2;
        //on even `counter`, create a new entity with a random position and insert it into the quadtree else move an existing entity to a new random position
        if counter % 2 == 0 {
            id = rand::random::<u32>();
            pos = Vec2 {
                x: rand::random::<f64>() * WORLD_SIZE - WORLD_SIZE / 2.0,
                y: rand::random::<f64>() * WORLD_SIZE - WORLD_SIZE / 2.0,
            };
        }
        else {
            if entities.is_empty() {
                counter += 1;
                continue;
            }

            id = *entities.keys().next().unwrap();
            pos = Vec2 {
                x: rand::random::<f64>() * WORLD_SIZE - WORLD_SIZE / 2.0,
                y: rand::random::<f64>() * WORLD_SIZE - WORLD_SIZE / 2.0,
            };
        }


        //if the entity already exists, verify if it has moved to a different shard
        if let Some(old_pos) = entities.get(&id) {
            let old_shard = quadtree.shard_for(*old_pos);
            let new_shard = quadtree.shard_for(pos);
            if old_shard != new_shard {
                // Handle entity moving to a different shard
                // envoyer `Unsubscribe(ancien topic)` puis `Subscribe(nouveau topic)` au broker
                println!("Entity {} moved from shard {:?} to shard {:?}", id, old_shard, new_shard);

                entities.insert(id, pos);
            }

            let nearby_shards = quadtree.shards_near(pos, NEARBY_MARGIN);
            if nearby_shards.len() > 1 {
                //émettre un `CrossingAlert`
                println!("Entity {} is near shard boundaries: nearby shards = {:?}", id, nearby_shards);
            }
        }

        // If it's a new entity, just insert it into the quadtree and track its position
        else {
            entities.insert(id, pos);
            println!("Entity {} created at position ({:.2}, {:.2}) in shard {:?}", id, pos.x, pos.y, quadtree.shard_for(pos));
        }

        //recreate the quadtree from scratch to simulate dynamic entity movement and shard changes
        let mut new_quadtree = Quadtree::new(boundary, 0);
        for (_id, pos) in &entities {
            new_quadtree.insert(*pos);
        }
        quadtree = new_quadtree;

        // Query Shard Data that will be sent to the orchestrator to update server layout
        let shard_data = quadtree.collect_shards();
        // Check if the shard layout has changed and if so, update the `shards` set and print the new layout
        let new_shard_ids: std::collections::HashSet<u32> = shard_data.iter().filter_map(|s| s.shard_id).collect();
        if new_shard_ids != shards {
            println!("Shard layout changed: new shard IDs = {:?}", new_shard_ids);
            shards = new_shard_ids;
            // Here we would send the new shard layout to the orchestrator, e.g. via a message broker
        }
        //print shard IDs and boundaries for debugging
        println!("Current Shard Layout:");
        for shard in &shard_data {
            println!("Shard ID: {:?}, Boundary: center=({:.2}, {:.2}), half_size={:.2}", shard.shard_id, shard.boundary.x, shard.boundary.y, shard.boundary.half_size);
        }

        counter += 1;

        // Sleep to simulate time passing
        std::thread::sleep(std::time::Duration::from_millis(1000));
    }  
}

#[derive(Debug, Clone, Copy)]
struct Vec2 {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct Boundary {
    x: f64,
    y: f64,
    half_size: f64,
}

impl Boundary {
    fn contains(&self, e: &Vec2) -> bool {
        let left = self.x - self.half_size;
        let right = self.x + self.half_size;
        let top = self.y - self.half_size;
        let bottom = self.y + self.half_size;

        e.x >= left && e.x < right &&
        e.y >= top  && e.y < bottom
    }

    fn quadrant(&self, e: &Vec2) -> Quadrant {
        if e.x >= self.x {
            if e.y < self.y {
                Quadrant::NorthEast
            } else {
                Quadrant::SouthEast
            }
        } else {
            if e.y < self.y {
                Quadrant::NorthWest
            } else {
                Quadrant::SouthWest
            }
        }
    }

    fn subdivide(&self) -> [Boundary; 4] {
        let hs = self.half_size / 2.0;
        [
            Boundary { x: self.x + hs, y: self.y - hs, half_size: hs }, // NE
            Boundary { x: self.x - hs, y: self.y - hs, half_size: hs }, // NW
            Boundary { x: self.x + hs, y: self.y + hs, half_size: hs }, // SE
            Boundary { x: self.x - hs, y: self.y + hs, half_size: hs }, // SW
        ]
    }

    fn intersects_range(&self, pos: Vec2, margin: f64) -> bool {
        let self_left = self.x - self.half_size;
        let self_right = self.x + self.half_size;
        let self_top = self.y - self.half_size;
        let self_bottom = self.y + self.half_size;

        let range_left = pos.x - margin;
        let range_right = pos.x + margin;
        let range_top = pos.y - margin;
        let range_bottom = pos.y + margin;

        // Returns true if the two AABBs overlap
        self_left < range_right && self_right > range_left &&
        self_top < range_bottom && self_bottom > range_top
    }
}

enum Quadrant {
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

#[derive(Debug)]
struct ShardData {
    shard_id: Option<u32>,
    boundary: Boundary,
}

struct Quadtree {
    boundary: Boundary,
    points: Vec<Vec2>,
    depth: u8,
    max_depth: u8,
    children: Option<[Box<Quadtree>; 4]>,
    shard_id: Option<u32>,  // défini uniquement sur les feuilles
}

impl Quadtree {
    fn new(boundary: Boundary, depth: u8) -> Self {
        Self {
            boundary,
            points: Vec::new(),
            depth,
            max_depth: MAX_DEPTH,
            children: None,
            shard_id: Some(0),
        }
    }

    fn insert(&mut self, point: Vec2) {
        if !self.boundary.contains(&point) {
            return;
        }

        if self.children.is_none() {
            if self.points.len() < MAX_CAPACITY {
                self.points.push(point);
                return;
            }

            // For now, goes betond max. Should implement phasing
            if self.depth >= self.max_depth {
                self.points.push(point);
                return;
            }

            self.split();
        }

        self.insert_into_child(point);
    }

    fn split(&mut self) {
        let boundaries = self.boundary.subdivide();

        let mut children = [
            Box::new(Quadtree::new(boundaries[0], self.depth + 1)),
            Box::new(Quadtree::new(boundaries[1], self.depth + 1)),
            Box::new(Quadtree::new(boundaries[2], self.depth + 1)),
            Box::new(Quadtree::new(boundaries[3], self.depth + 1)),
        ];

        if self.depth == 0 {
            // Assign shard IDs to top-level quadrants
            for (i, child) in children.iter_mut().enumerate() {
                child.shard_id = Some((i + 1) as u32); // Simple shard ID assignment
            }
        }else {
            // Shard ID = parent shard ID + quadrant index
            for (i, child) in children.iter_mut().enumerate() {
                child.shard_id = self.shard_id.map(|id| id * 10 + (i + 1) as u32); // Simple hierarchical shard ID
            }

            self.shard_id = None; // Clear parent shard ID since it's no longer a leaf
        }

        let old = mem::take(&mut self.points);
        for e in old {
            let idx = match self.boundary.quadrant(&e) {
                Quadrant::NorthEast => 0,
                Quadrant::NorthWest => 1,
                Quadrant::SouthEast => 2,
                Quadrant::SouthWest => 3,
            };
            children[idx].insert(e);
        }

        self.children = Some(children);
    }

    fn insert_into_child(&mut self, point: Vec2) {
        let idx = match self.boundary.quadrant(&point) {
            Quadrant::NorthEast => 0,
            Quadrant::NorthWest => 1,
            Quadrant::SouthEast => 2,
            Quadrant::SouthWest => 3,
        };

        self.children.as_mut().unwrap()[idx].insert(point);
    }

    fn collect_shards(&self) -> Vec<ShardData> {
        let mut out = Vec::new();
        self.collect_into(&mut out);
        out
    }

    fn collect_into(&self, out: &mut Vec<ShardData>) {
        match &self.children {
            None => {
                // If it's a leaf, it IS a shard, even if empty.
                out.push(ShardData {
                    boundary: self.boundary,
                    shard_id: self.shard_id,
                });
            }
            Some(children) => {
                for c in children {
                    c.collect_into(out);
                }
            }
        }
    }

    /// Retourne le shard_id de la feuille contenant `pos`.
    pub fn shard_for(&self, pos: Vec2) -> Option<u32> {
        if !self.boundary.contains(&pos) {
            return None;
        }

        match &self.children {
            None => self.shard_id,
            Some(children) => {
                // Instantly pinpoint the correct quadrant index
                let idx = match self.boundary.quadrant(&pos) {
                    Quadrant::NorthEast => 0,
                    Quadrant::NorthWest => 1,
                    Quadrant::SouthEast => 2,
                    Quadrant::SouthWest => 3,
                };
                children[idx].shard_for(pos)
            }
        }
    }

    /// Retourne les shard_ids distincts dans un rayon `margin` autour de `pos`.
    /// Utilisé pour détecter l'approche d'une frontière inter-shard.
    pub fn shards_near(&self, pos: Vec2, margin: f64) -> Vec<u32> { 
        let mut shards = Vec::new();
        self.collect_shards_near(pos, margin, &mut shards);
        shards.sort_unstable();
        shards.dedup();
        shards
    }

    fn collect_shards_near(&self, pos: Vec2, margin: f64, shards: &mut Vec<u32>) {
        if !self.boundary.intersects_range(pos, margin) {
            return;
        }

        match &self.children {
            None => {
                if let Some(shard_id) = self.shard_id {
                    shards.push(shard_id);
                }
            }
            Some(children) => {
                for c in children {
                    c.collect_shards_near(pos, margin, shards);
                }
            }
        }
    }
}