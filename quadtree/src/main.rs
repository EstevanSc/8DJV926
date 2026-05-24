use std::mem;

const MAX_CAPACITY: usize = 4;

fn main() {
    let boundary = Boundary {
        x: 0.0,
        y: 0.0,
        half_size: 10.0,
    };

    let mut quadtree = Quadtree::new(boundary);

    let entities = vec![
        Entity { id: 1, x: -5.0, y: -5.0 },
        Entity { id: 2, x:  5.0, y: -5.0 },
        Entity { id: 3, x: -5.0, y:  5.0 },
        Entity { id: 4, x:  5.0, y:  5.0 },
        Entity { id: 5, x:  0.0, y:  0.0 },
    ];

    for e in entities {
        quadtree.insert(e);
    }

    // What you transmit
    let packets = quadtree.collect_quadrants();

    for q in packets {
        println!(
            "Quadrant center=({}, {}), half_size={}, entities={}",
            q.boundary.x,
            q.boundary.y,
            q.boundary.half_size,
            q.entities.len()
        );
        for e in q.entities {
            println!("  Entity {} at ({}, {})", e.id, e.x, e.y);
        }
    }
}

#[derive(Debug, Clone)]
struct Entity {
    id: u32,
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
    fn contains(&self, e: &Entity) -> bool {
        let left = self.x - self.half_size;
        let right = self.x + self.half_size;
        let top = self.y - self.half_size;
        let bottom = self.y + self.half_size;

        e.x >= left && e.x < right &&
        e.y >= top  && e.y < bottom
    }

    fn quadrant(&self, e: &Entity) -> Quadrant {
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
}

enum Quadrant {
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

struct QuadrantData {
    boundary: Boundary,
    entities: Vec<Entity>,
}

struct Quadtree {
    boundary: Boundary,
    entities: Vec<Entity>, // only used until split
    children: Option<[Box<Quadtree>; 4]>,
}

impl Quadtree {
    fn new(boundary: Boundary) -> Self {
        Self {
            boundary,
            entities: Vec::new(),
            children: None,
        }
    }

    fn insert(&mut self, entity: Entity) {
        if !self.boundary.contains(&entity) {
            return;
        }

        if self.children.is_none() {
            if self.entities.len() < MAX_CAPACITY {
                self.entities.push(entity);
                return;
            }

            self.split();
        }

        self.insert_into_child(entity);
    }

    fn split(&mut self) {
        let boundaries = self.boundary.subdivide();

        let mut children = [
            Box::new(Quadtree::new(boundaries[0])),
            Box::new(Quadtree::new(boundaries[1])),
            Box::new(Quadtree::new(boundaries[2])),
            Box::new(Quadtree::new(boundaries[3])),
        ];

        let old = mem::take(&mut self.entities);
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

    fn insert_into_child(&mut self, entity: Entity) {
        let idx = match self.boundary.quadrant(&entity) {
            Quadrant::NorthEast => 0,
            Quadrant::NorthWest => 1,
            Quadrant::SouthEast => 2,
            Quadrant::SouthWest => 3,
        };

        self.children.as_mut().unwrap()[idx].insert(entity);
    }

    /// ✅ What you serialize & transmit
    fn collect_quadrants(&self) -> Vec<QuadrantData> {
        let mut out = Vec::new();
        self.collect_into(&mut out);
        out
    }

    fn collect_into(&self, out: &mut Vec<QuadrantData>) {
        match &self.children {
            None => {
                if !self.entities.is_empty() {
                    out.push(QuadrantData {
                        boundary: self.boundary,
                        entities: self.entities.clone(), // clone only at serialization boundary
                    });
                }
            }
            Some(children) => {
                for c in children {
                    c.collect_into(out);
                }
            }
        }
    }
}