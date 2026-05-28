mod quic_client;

use common::{ShardData, Boundary, Vec2, Quadrant};
use quic_client::QuicClient;
use std::mem;

/// Load configuration from environment variables with defaults.
struct Config {
    world_size: f64,
    max_capacity: usize,
    max_depth: u8,
    nearby_margin: f64,
    orchestrator_host: String,
    orchestrator_port: u16,
    broker_host: String,
    broker_port: u16,
    entity_add_interval_ms: u64,
}

impl Config {
    fn from_env() -> Self {
        dotenv::dotenv().ok();

        Config {
            world_size: std::env::var("QUADTREE_WORLD_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            max_capacity: std::env::var("QUADTREE_MAX_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(4),
            max_depth: std::env::var("QUADTREE_MAX_DEPTH")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            nearby_margin: std::env::var("QUADTREE_NEARBY_MARGIN")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5.0),
            orchestrator_host: std::env::var("QUADTREE_ORCHESTRATOR_HOST")
                .unwrap_or_else(|_| "localhost".to_string()),
            orchestrator_port: std::env::var("QUADTREE_ORCHESTRATOR_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5000),
            broker_host: std::env::var("QUADTREE_BROKER_HOST")
                .unwrap_or_else(|_| "broker".to_string()),
            broker_port: std::env::var("QUADTREE_BROKER_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(7776),
            entity_add_interval_ms: std::env::var("QUADTREE_ENTITY_ADD_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1000),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    tracing::info!("Quadtree starting with config: world_size={}, max_capacity={}, max_depth={}, nearby_margin={}",
        config.world_size, config.max_capacity, config.max_depth, config.nearby_margin);

    // Connect to orchestrator and broker via separate QUIC links.
    let orchestrator_client = match QuicClient::connect_orchestrator(&config.orchestrator_host, config.orchestrator_port).await {
        Ok(client) => {
            tracing::info!("Connected to orchestrator at {}:{}", config.orchestrator_host, config.orchestrator_port);
            Some(client)
        }
        Err(e) => {
            tracing::error!("Failed to connect to orchestrator: {}. Running without QUIC updates.", e);
            None
        }
    };

    let broker_client = match QuicClient::connect_broker(&config.broker_host, config.broker_port).await {
        Ok(client) => {
            tracing::info!("Connected to broker at {}:{}", config.broker_host, config.broker_port);
            Some(client)
        }
        Err(e) => {
            tracing::warn!("Failed to connect to broker at {}:{}: {}", config.broker_host, config.broker_port, e);
            None
        }
    };

    run_main_loop(config, orchestrator_client, broker_client).await
}

async fn run_main_loop(
    config: Config,
    orchestrator_client: Option<QuicClient>,
    _broker_client: Option<QuicClient>,
) -> anyhow::Result<()> {
    let boundary = Boundary {
        x: 0.0,
        y: 0.0,
        half_size: config.world_size / 2.0,
    };

    let mut quadtree = Quadtree::new(boundary, 0, config.max_depth, config.max_capacity);

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
                x: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
                y: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
            };
        }
        else {
            if entities.is_empty() {
                counter += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(config.entity_add_interval_ms)).await;
                continue;
            }

            id = *entities.keys().next().unwrap();
            pos = Vec2 {
                x: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
                y: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
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

            let nearby_shards = quadtree.shards_near(pos, config.nearby_margin);
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
        let mut new_quadtree = Quadtree::new(boundary, 0, config.max_depth, config.max_capacity);
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
            
            // Send shard layout to orchestrator via QUIC
            if let Some(ref client) = orchestrator_client {
                if let Err(e) = client.send_shard_data(&shard_data).await {
                    tracing::error!("Failed to send shard data to orchestrator: {}", e);
                }
            }
        }
        //print shard IDs and boundaries for debugging
        println!("Current Shard Layout:");
        for shard in &shard_data {
            println!("Shard ID: {:?}, Boundary: center=({:.2}, {:.2}), half_size={:.2}", shard.shard_id, shard.boundary.x, shard.boundary.y, shard.boundary.half_size);
        }

        counter += 1;

        // Sleep to simulate time passing
        tokio::time::sleep(tokio::time::Duration::from_millis(config.entity_add_interval_ms)).await;
    }  
}

struct Quadtree {
    boundary: Boundary,
    points: Vec<Vec2>,
    depth: u8,
    max_depth: u8,
    max_capacity: usize,
    children: Option<[Box<Quadtree>; 4]>,
    shard_id: Option<u32>,  // défini uniquement sur les feuilles
}

impl Quadtree {
    fn new(boundary: Boundary, depth: u8, max_depth: u8, max_capacity: usize) -> Self {
        Self {
            boundary,
            points: Vec::new(),
            depth,
            max_depth,
            max_capacity,
            children: None,
            shard_id: Some(0),
        }
    }

    fn insert(&mut self, point: Vec2) {
        if !self.boundary.contains(&point) {
            return;
        }

        if self.children.is_none() {
            if self.points.len() < self.max_capacity {
                self.points.push(point);
                return;
            }

            // For now, goes beyond max. Should implement phasing
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
            Box::new(Quadtree::new(boundaries[0], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[1], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[2], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[3], self.depth + 1, self.max_depth, self.max_capacity)),
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