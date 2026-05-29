mod quic_client;

use common::broker_messages::BrokerMessage;
use common::packets::PositionBatch;
use common::topics::{
    deserialize_position_payload, deserialize_shard_created_payload,
    deserialize_shard_snapshot_payload, PositionPayload, ShardCreatedPayload, Topic,
};
use common::{Boundary, Quadrant, ShardData, Vec2};
use game_sockets::GameNetworkEvent;
use quic_client::QuicClient;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::LazyLock;
use uuid::Uuid;

static QUADTREE_ID: LazyLock<Uuid> = LazyLock::new(Uuid::new_v4);

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

fn rebuild_quadtree(
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    entity_positions: &HashMap<Uuid, Vec2>,
) -> Quadtree {
    let mut quadtree = Quadtree::new(boundary, 0, max_depth, max_capacity);

    for position in entity_positions.values() {
        quadtree.insert(*position);
    }

    quadtree
}

fn entity_id_from_uuid(id: Uuid) -> u32 {
    id.as_bytes()
        .iter()
        .fold(0u32, |acc, &byte| acc.wrapping_add(byte as u32))
}

async fn subscribe_entity_input(
    broker_client: &QuicClient,
    shard_uuid: Uuid,
    entity_id: Uuid,
) -> anyhow::Result<()> {
    broker_client
        .subscribe(shard_uuid, Topic::Input(entity_id))
        .await?;

    Ok(())
}

async fn handle_position_payload(
    broker_client: Option<&QuicClient>,
    payload: PositionPayload,
    quadtree: &Quadtree,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &HashMap<u32, Uuid>,
) -> anyhow::Result<()> {
    entity_positions.insert(payload.entity_id, payload.position);

    let Some(shard_id) = quadtree.shard_for(payload.position) else {
        return Ok(());
    };

    let previous_shard_id = entity_shard_ids.insert(payload.entity_id, shard_id);

    if let Some(client) = broker_client {
        if let Some(previous_shard_id) = previous_shard_id {
            if previous_shard_id != shard_id {
                if let Some(previous_shard_uuid) = shard_uuid_by_id.get(&previous_shard_id) {
                    client
                        .unsubscribe(*previous_shard_uuid, Topic::Input(payload.entity_id))
                        .await?;
                }
            }
        }

        if let Some(shard_uuid) = shard_uuid_by_id.get(&shard_id) {
            subscribe_entity_input(client, *shard_uuid, payload.entity_id).await?;
            client
                .subscribe(payload.entity_id, Topic::ShardSnapshot(*shard_uuid))
                .await?;
        }
    }

    Ok(())
}

async fn handle_shard_created_payload(
    broker_client: Option<&QuicClient>,
    payload: ShardCreatedPayload,
    quadtree: &Quadtree,
    entity_positions: &HashMap<Uuid, Vec2>,
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
) -> anyhow::Result<()> {
    let Some(shard_id) = quadtree.shard_for(payload.center) else {
        return Ok(());
    };

    shard_uuid_by_id.insert(shard_id, payload.shard_id);

    if let Some(client) = broker_client {
        client
            .subscribe(*QUADTREE_ID, Topic::ShardSnapshot(payload.shard_id))
            .await?;

        let entity_ids_in_shard: Vec<Uuid> = entity_positions
            .iter()
            .filter_map(|(entity_id, position)| {
                (quadtree.shard_for(*position) == Some(shard_id)).then_some(*entity_id)
            })
            .collect();

        for entity_id in entity_ids_in_shard {
            // shard subscribe to entity input for entities in the shard
            subscribe_entity_input(client, payload.shard_id, entity_id).await?;
            // entity subscribe to shard snapshot to receive shard layout updates
            client.subscribe( entity_id, Topic::ShardSnapshot(payload.shard_id)).await?;
        }
    }

    Ok(())
}

async fn handle_shard_snapshot_payload(
    payload: Vec<u8>,
    quadtree: &mut Quadtree,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shards: &mut HashSet<u32>,
) -> anyhow::Result<()> {
    let snapshot = deserialize_shard_snapshot_payload(&payload)
        .ok_or_else(|| anyhow::anyhow!("Failed to decode shard snapshot payload"))?;

    let batch = wincode::deserialize::<PositionBatch>(&snapshot.replication)
        .map_err(|_| anyhow::anyhow!("Failed to decode shard snapshot replication batch"))?;

    let entity_uuid_by_id: HashMap<u32, Uuid> = entity_positions
        .keys()
        .copied()
        .map(|uuid| (entity_id_from_uuid(uuid), uuid))
        .collect();

    for snap in batch.snapshots {
        if let Some(entity_uuid) = entity_uuid_by_id.get(&snap.entity_id).copied() {
            entity_positions.insert(
                entity_uuid,
                Vec2 {
                    x: snap.x as f64,
                    y: snap.y as f64,
                },
            );
        }
    }

    *quadtree = rebuild_quadtree(boundary, max_depth, max_capacity, entity_positions);

    entity_shard_ids.clear();
    for (entity_id, position) in entity_positions.iter() {
        if let Some(shard_id) = quadtree.shard_for(*position) {
            entity_shard_ids.insert(*entity_id, shard_id);
        }
    }

    let new_shard_ids: HashSet<u32> = quadtree
        .collect_shards()
        .into_iter()
        .filter_map(|shard| shard.shard_id)
        .collect();
    *shards = new_shard_ids;

    Ok(())
}

async fn handle_broker_message(
    broker_client: Option<&QuicClient>,
    message: BrokerMessage,
    quadtree: &mut Quadtree,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    shards: &mut HashSet<u32>,
) -> anyhow::Result<()> {
    match message {
        BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
            Topic::Position => {
                let payload = deserialize_position_payload(&payload)
                    .ok_or_else(|| anyhow::anyhow!("Failed to decode position payload"))?;
                tracing::info!("Quadtree received Position for {}", payload.entity_id);
                handle_position_payload(
                    broker_client,
                    payload,
                    quadtree,
                    entity_positions,
                    entity_shard_ids,
                    shard_uuid_by_id,
                )
                .await?;
            }
            Topic::ShardCreated => {
                let payload = deserialize_shard_created_payload(&payload)
                    .ok_or_else(|| anyhow::anyhow!("Failed to decode shard created payload"))?;
                tracing::info!("Quadtree received ShardCreated for shard {}", payload.shard_id);
                handle_shard_created_payload(
                    broker_client,
                    payload,
                    quadtree,
                    entity_positions,
                    shard_uuid_by_id,
                )
                .await?;
            }
            Topic::ShardSnapshot(_) => {
                tracing::info!("Quadtree received ShardSnapshot");
                handle_shard_snapshot_payload(
                    payload,
                    quadtree,
                    boundary,
                    max_depth,
                    max_capacity,
                    entity_positions,
                    entity_shard_ids,
                    shards,
                )
                .await?;
            }
            _ => {}
        },
        _ => {}
    }

    Ok(())
}

async fn process_broker_events(
    broker_client: &mut QuicClient,
    quadtree: &mut Quadtree,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    shards: &mut HashSet<u32>,
) -> anyhow::Result<()> {
    loop {
        let Some(event) = broker_client.poll()? else {
            break;
        };

        match event {
            GameNetworkEvent::Message { data, .. } => {
                if let Some(message) = BrokerMessage::deserialize(&data) {
                    handle_broker_message(
                        Some(broker_client),
                        message,
                        quadtree,
                        entity_positions,
                        entity_shard_ids,
                        shard_uuid_by_id,
                        boundary,
                        max_depth,
                        max_capacity,
                        shards,
                    )
                    .await?;
                }
            }
            GameNetworkEvent::Disconnected(connection) => {
                tracing::warn!("Broker disconnected ({:?})", connection.connection_id);
                break;
            }
            GameNetworkEvent::Error { inner, .. } => {
                tracing::warn!("Broker network error: {inner}");
                break;
            }
            _ => {}
        }
    }

    Ok(())
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

    let quadtree_id = *QUADTREE_ID;

    if let Some(client) = broker_client.as_ref() {
        if let Err(e) = client.announce_connect(quadtree_id).await {
            tracing::warn!("Failed to connect to broker: {e}");
        }
        if let Err(e) = client.subscribe(quadtree_id, Topic::Position).await {
            tracing::warn!("Failed to subscribe to Position: {e}");
        } else {
            tracing::info!("Quadtree subscribed to Position");
        }
        if let Err(e) = client.subscribe(quadtree_id, Topic::ShardCreated).await {
            tracing::warn!("Failed to subscribe to ShardCreated: {e}");
        } else {
            tracing::info!("Quadtree subscribed to ShardCreated");
        }
    }

    run_main_loop(config, orchestrator_client, broker_client, quadtree_id).await
}

async fn run_main_loop(
    config: Config,
    orchestrator_client: Option<QuicClient>,
    mut broker_client: Option<QuicClient>,
    _broker_client_id: Uuid,
) -> anyhow::Result<()> {
    let boundary = Boundary {
        x: 0.0,
        y: 0.0,
        half_size: config.world_size / 2.0,
    };

    let mut quadtree = Quadtree::new(boundary, 0, config.max_depth, config.max_capacity);

    let mut entity_positions: HashMap<Uuid, Vec2> = HashMap::new();
    let mut entity_shard_ids: HashMap<Uuid, u32> = HashMap::new();
    let mut shard_uuid_by_id: HashMap<u32, Uuid> = HashMap::new();

    let mut shards = HashSet::new();

    let mut counter = 0;

    //simulate a publish-subscribe system where entities are added to the quadtree and we query for nearby shards
    loop {
        if let Some(client) = broker_client.as_mut() {
            process_broker_events(
                client,
                &mut quadtree,
                &mut entity_positions,
                &mut entity_shard_ids,
                &mut shard_uuid_by_id,
                boundary,
                config.max_depth,
                config.max_capacity,
                &mut shards,
            )
            .await?;
        }
        /*
        // Simulate entity creation

        let id: Uuid;
        let pos: Vec2;
        //on even `counter`, create a new entity with a random position and insert it into the quadtree else move an existing entity to a new random position
        if counter % 2 == 0 {
            id = Uuid::new_v4();
            pos = Vec2 {
                x: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
                y: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
            };
        }
        else {
            if entity_positions.is_empty() {
                counter += 1;
                tokio::time::sleep(tokio::time::Duration::from_millis(config.entity_add_interval_ms)).await;
                continue;
            }

            id = *entity_positions.keys().next().unwrap();
            pos = Vec2 {
                x: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
                y: rand::random::<f64>() * config.world_size - config.world_size / 2.0,
            };
        }
        

        //if the entity already exists, verify if it has moved to a different shard
        if let Some(old_pos) = entity_positions.get(&id) {
            let old_shard = quadtree.shard_for(*old_pos);
            let new_shard = quadtree.shard_for(pos);
            if old_shard != new_shard {
                // Handle entity moving to a different shard
                // envoyer `Unsubscribe(ancien topic)` puis `Subscribe(nouveau topic)` au broker
                println!("Entity {} moved from shard {:?} to shard {:?}", id, old_shard, new_shard);

                entity_positions.insert(id, pos);
            }

            let nearby_shards = quadtree.shards_near(pos, config.nearby_margin);
            if nearby_shards.len() > 1 {
                //émettre un `CrossingAlert`
                println!("Entity {} is near shard boundaries: nearby shards = {:?}", id, nearby_shards);
            }
        }
        
        // If it's a new entity, just insert it into the quadtree and track its position
        else {
            entity_positions.insert(id, pos);
            println!("Entity {} created at position ({:.2}, {:.2}) in shard {:?}", id, pos.x, pos.y, quadtree.shard_for(pos));
        }
        */
        //recreate the quadtree from scratch to simulate dynamic entity movement and shard changes
        quadtree = rebuild_quadtree(boundary, config.max_depth, config.max_capacity, &entity_positions);

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
        //println!("Current Shard Layout:");
        //for shard in &shard_data {
        //    println!("Shard ID: {:?}, Boundary: center=({:.2}, {:.2}), half_size={:.2}", shard.shard_id, shard.boundary.x, shard.boundary.y, shard.boundary.half_size);
        //}

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