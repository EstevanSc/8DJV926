mod quic_client;
use common::{Boundary, Quadrant};
use common::topics::{
    deserialize_shard_created_payload, Topic,
    deserialize_position_payload, serialize_position_payload, PositionPayload
};
use common::BrokerMessage;
use game_sockets::GameNetworkEvent;
use quic_client::QuicClient;
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

type SharedShardSet = Arc<RwLock<HashSet<Boundary>>>;
type SharedShardMap = Arc<RwLock<HashMap<Boundary, Option<uuid::Uuid>>>>;
type SharedPendingPlayers = Arc<tokio::sync::Mutex<VecDeque<(Instant, PositionPayload)>>>;
type SharedEntityMap = Arc<RwLock<HashMap<uuid::Uuid, [f64; 2]>>>;

const PLAYER_SPAWN_RETRY_TIMEOUT_SECS: u64 = 1;

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


    let shard_set: SharedShardSet = Arc::new(RwLock::new(HashSet::new()));
    let shard_map: SharedShardMap = Arc::new(RwLock::new(HashMap::new()));
    let pending_players: SharedPendingPlayers = Arc::new(tokio::sync::Mutex::new(VecDeque::new()));
    let entity_map: SharedEntityMap = Arc::new(RwLock::new(HashMap::new()));

    run_main_loop(config, orchestrator_client, broker_client, shard_set, shard_map, pending_players, entity_map).await
}


async fn run_main_loop(
    config: Config,
    orchestrator_client: Option<QuicClient>,
    broker_client: Option<QuicClient>,
    shard_set: SharedShardSet,
    shard_map: SharedShardMap,
    pending_players: SharedPendingPlayers,
    entity_map: SharedEntityMap,
) -> anyhow::Result<()> {
    let boundary = Boundary {
        x: 0.0,
        y: 0.0,
        half_size: config.world_size / 2.0,
    };

    let mut quadtree = Quadtree::new_root(
        boundary,
        config.max_depth,
        config.max_capacity,
        shard_set.clone(),
        shard_map.clone(),
    );

    let orchestrator_client = orchestrator_client;
    let mut broker_client = broker_client;

    if let Some(client) = broker_client.as_ref() {
        let client_id = client.connection_id();
        client.announce_connect(client_id).await?;
        client.subscribe(client_id, Topic::ShardCreated).await?;
        client.subscribe(client_id, Topic::PlayerStartingPosition).await?;
        tracing::info!(
            "Subscribed quadtree to broker topics {:?} and {:?} with client_id={}",
            Topic::ShardCreated,
            Topic::PlayerStartingPosition,
            client_id
        );
    }

    //Send the initial shard configuration to the orchestrator
    if let Some(client) = orchestrator_client.as_ref() {
        send_server_configuration_update(&client, vec![boundary]).await?;
    } else {
        tracing::warn!("No connection to orchestrator, skipping initial shard configuration update");
    }
    let mut tick = tokio::time::interval(Duration::from_millis(config.entity_add_interval_ms));

    loop {
        tick.tick().await;

        let mut flagged_for_rebuild = false;

        if let Some(client) = broker_client.as_mut() {
              poll_quic_events(client, "broker", &shard_map, &pending_players, &entity_map, &mut flagged_for_rebuild)?;
        }

        if let Some(client) = broker_client.as_ref() {
              process_pending_players(&pending_players, client, &shard_map, &entity_map, &mut flagged_for_rebuild).await;
        }

        if flagged_for_rebuild {
            tracing::info!("Rebuilding quadtree due to shard changes...");

            let old_shard_set = shard_set.read().unwrap().clone();

            let points: Vec<[f64; 2]> = {
                let entity_map = entity_map.read().unwrap();
                entity_map.values().copied().collect()
            };

            quadtree.rebuild(boundary, points);

            let new_shard_set = shard_set.read().unwrap().clone();
            if new_shard_set != old_shard_set {
                tracing::info!("Shard set changed after rebuild, sending updated configuration to orchestrator...");
                if let Some(client) = orchestrator_client.as_ref() {
                    send_server_configuration_update(client, new_shard_set.into_iter().collect()).await?;
                } else {
                    tracing::warn!("No connection to orchestrator, skipping shard configuration update");
                }
            }
        }
    }
}

fn poll_quic_events(
    client: &mut QuicClient,
    label: &str,
    shard_map: &SharedShardMap,
    pending_players: &SharedPendingPlayers,
    entity_map: &SharedEntityMap,
    flagged_for_rebuild: &mut bool,
) -> anyhow::Result<()> {
    while let Some(event) = client.poll()? {
        match event {
            GameNetworkEvent::Message { data, connection, stream } => {
                tracing::debug!(
                    "{} link message: {} bytes from {:?} on stream {}",
                    label,
                    data.len(),
                    connection.connection_id,
                    stream.stream_id
                );

                handle_quic_message(&data, shard_map, pending_players, entity_map, flagged_for_rebuild);
            }
            _ => {}
        }
    }

    Ok(())
}

struct Quadtree {
    boundary: Boundary,
    points: Vec<[f64; 2]>,
    depth: u8,
    max_depth: u8,
    max_capacity: usize,
    children: Option<[Box<Quadtree>; 4]>,
    shard_set: SharedShardSet,
    shard_map: SharedShardMap,
}

impl Quadtree {
    fn new(
        boundary: Boundary,
        depth: u8,
        max_depth: u8,
        max_capacity: usize,
        shard_set: SharedShardSet,
        shard_map: SharedShardMap,
    ) -> Self {
        Self {
            boundary,
            points: Vec::new(),
            depth,
            max_depth,
            max_capacity,
            children: None,
            shard_set: shard_set.clone(),
            shard_map: shard_map.clone(),
        }
    }

    fn new_root(
        boundary: Boundary,
        max_depth: u8,
        max_capacity: usize,
        shard_set: SharedShardSet,
        shard_map: SharedShardMap,
    ) -> Self {
        {
            let mut set = shard_set.write().unwrap();
            set.clear();
            set.insert(boundary);
        }

        {
            let mut map = shard_map.write().unwrap();
            map.clear();
            map.insert(boundary, None);
        }

        Self::new(boundary, 0, max_depth, max_capacity, shard_set, shard_map)
    }

    fn insert(&mut self, point: [f64; 2]) {
        if !self.boundary.contains(&point[0], &point[1]) {
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

        {
            let mut set = self.shard_set.write().unwrap();
            set.remove(&self.boundary);
            for b in boundaries {
                set.insert(b);
            }
        }
        {
            let mut map = self.shard_map.write().unwrap();
            map.remove(&self.boundary);
            for b in boundaries {
                map.entry(b).or_insert(None);
            }
        }

        let mut children = [
            Box::new(Quadtree::new(boundaries[0], self.depth + 1, self.max_depth, self.max_capacity, self.shard_set.clone(), self.shard_map.clone())),
            Box::new(Quadtree::new(boundaries[1], self.depth + 1, self.max_depth, self.max_capacity, self.shard_set.clone(), self.shard_map.clone())),
            Box::new(Quadtree::new(boundaries[2], self.depth + 1, self.max_depth, self.max_capacity, self.shard_set.clone(), self.shard_map.clone())),
            Box::new(Quadtree::new(boundaries[3], self.depth + 1, self.max_depth, self.max_capacity, self.shard_set.clone(), self.shard_map.clone())),
        ];

        let old = mem::take(&mut self.points);
        for point in old {
            let idx = match self.boundary.quadrant(&point[0], &point[1]) {
                Quadrant::NorthEast => 0,
                Quadrant::NorthWest => 1,
                Quadrant::SouthEast => 2,
                Quadrant::SouthWest => 3,
            };
            children[idx].insert(point);
        }

        self.children = Some(children);
    }

    fn insert_into_child(&mut self, point: [f64; 2]) {
        let idx = match self.boundary.quadrant(&point[0], &point[1]) {
            Quadrant::NorthEast => 0,
            Quadrant::NorthWest => 1,
            Quadrant::SouthEast => 2,
            Quadrant::SouthWest => 3,
        };

        self.children.as_mut().unwrap()[idx].insert(point);
    }

    fn rebuild(&mut self, boundary: Boundary, points: Vec<[f64; 2]>) {

        self.points.clear();
        self.points.extend(points);
        self.children = None;

        {
            let mut set = self.shard_set.write().unwrap();
            set.clear();
            set.insert(boundary);
        }

        {
            let mut map = self.shard_map.write().unwrap();
            map.clear();
            map.insert(boundary, None);
        }

        self.split();
    }
}

async fn send_server_configuration_update(client: &QuicClient, boundaries: Vec<Boundary>) -> anyhow::Result<()> {
    client.send_shard_data(&boundaries).await?;
    Ok(())
}

fn handle_quic_message(data: &[u8], shard_map: &SharedShardMap, pending_players: &SharedPendingPlayers, entity_map: &SharedEntityMap, flagged_for_rebuild: &mut bool) {
    let Some(message) = common::BrokerMessage::deserialize(data) else {
        return;
    };

    match message {
        common::BrokerMessage::Broadcast { topic, payload }
        | common::BrokerMessage::Publish { topic, payload } => {
            match Topic::from_bytes(topic) {
                Topic::PlayerStartingPosition => {
                    handle_player_starting_position_topic(&payload, pending_players);
                }
                Topic::ShardCreated => handle_shard_created_topic(&payload, shard_map),
                Topic::EntityPositionUpdate(_) => handle_entity_position_update_topic(&payload, shard_map, entity_map, flagged_for_rebuild),
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_shard_created_topic(payload: &[u8], shard_map: &SharedShardMap) {
    let Some(parsed) = deserialize_shard_created_payload(payload) else {
        return;
    };

    let mut map = shard_map.write().unwrap();
    map.insert(parsed.boundary, Some(parsed.shard_connection_id));
    tracing::info!(
        "Shard registered: shard_uuid={} boundary=({}, {}, {})",
        parsed.shard_connection_id, parsed.boundary.x, parsed.boundary.y, parsed.boundary.half_size
    );
}

fn handle_player_starting_position_topic(payload: &[u8], pending_players: &SharedPendingPlayers) {
    if let Some(parsed) = deserialize_position_payload(payload) {
        handle_player_starting_position_payload(parsed, pending_players);
    }
}

fn handle_player_starting_position_payload(
    payload: PositionPayload,
    pending_players: &SharedPendingPlayers,
) {
    match pending_players.try_lock() {
        Ok(mut pending) => {
            tracing::debug!(
                "Queuing player spawn for player_id={} at ({}, {})",
                payload.connection_id, payload.position[0], payload.position[1]
            );
            pending.push_back((Instant::now(), payload));
        }
        Err(_) => {
            tracing::warn!("Pending players queue contended, dropping spawn request for player_id={}", payload.connection_id);
        }
    }
}

/// Find the leaf shard boundary that spatially contains the given point.
fn find_shard_for_position(shard_map: &SharedShardMap, x: f64, y: f64) -> Option<(Boundary, Option<uuid::Uuid>)> {
    let map = shard_map.read().unwrap();
    map.iter()
        .find(|(boundary, _)| boundary.contains(&x, &y))
        .map(|(b, uuid)| (*b, *uuid))
}

/// Subscribe the shard server to the correct topics, then publish the starting position.
async fn spawn_player_on_shard(
    broker: &QuicClient,
    shard_uuid: uuid::Uuid,
    player_id: uuid::Uuid,
    position: [f64; 2],
    entity_map: &SharedEntityMap,
    flagged_for_rebuild: &mut bool
) -> anyhow::Result<()> {
    //print the ids for debugging
    tracing::info!("Spawning player player_id={} on shard shard_uuid={}", player_id, shard_uuid);
    tracing::info!("Quadtree connection_id={}", broker.connection_id());


    // Subscribe the shard server to player-specific inbound topics.
    broker.subscribe(shard_uuid, Topic::PlayerStartingPositionInShard(player_id)).await?;
    broker.subscribe(shard_uuid, Topic::Input(player_id)).await?;
    broker.subscribe(shard_uuid, Topic::Disconnect(player_id)).await?;
    
    //subscribe the client to the player's position updates so it can track its own position for interpolation
    broker.subscribe(player_id, Topic::EntityPositionUpdate(player_id)).await?;

    //send the initial position update so the client and quadtree have a baseline position for the player
     let payload = serialize_position_payload(&PositionPayload {
        connection_id: player_id,
        position,
    });

    broker.publish(Topic::EntityPositionUpdate(player_id), &payload).await?;   

    //subscribe the quadtree to the player's position updates so it can track which shard they are in
    broker.subscribe(broker.connection_id(), Topic::EntityPositionUpdate(player_id)).await?; 

    entity_map.write().unwrap().insert(player_id, position);

    let config = Config::from_env();

    if entity_map.read().unwrap().len() == config.max_capacity {
        *flagged_for_rebuild = true;
    }

    // Publish the starting position so the shard server spawns the player.
    let payload_bytes = serialize_position_payload(&PositionPayload { connection_id: player_id, position });
    broker.publish(Topic::PlayerStartingPositionInShard(player_id), &payload_bytes).await?;

    tracing::info!(
        "Spawned player player_id={}  on shard shard_uuid={}",
        player_id, shard_uuid
    );
    Ok(())
}

/// Drain the pending queue each tick: resolve players whose shard UUID is now known.
async fn process_pending_players(
    pending_players: &SharedPendingPlayers,
    broker: &QuicClient,
    shard_map: &SharedShardMap,
    entity_map: &SharedEntityMap,
    flagged_for_rebuild: &mut bool,
) {
    let timeout = Duration::from_secs(PLAYER_SPAWN_RETRY_TIMEOUT_SECS);
    let mut ready: Vec<(uuid::Uuid, uuid::Uuid, [f64; 2])> = Vec::new();
    let mut still_pending: VecDeque<(Instant, PositionPayload)> = VecDeque::new();

    {
        let mut pending = pending_players.lock().await;
        while let Some((queued_at, payload)) = pending.pop_front() {
            if queued_at.elapsed() > timeout {
                tracing::warn!(
                    "Player spawn timed out — dropping player_id={}",
                    payload.connection_id
                );
                continue;
            }

            match find_shard_for_position(shard_map, payload.position[0], payload.position[1]) {
                Some((_, Some(shard_uuid))) => {
                    ready.push((shard_uuid, payload.connection_id, payload.position));
                }
                _ => {
                    still_pending.push_back((queued_at, payload));
                }
            }
        }
        *pending = still_pending;
    }

    for (shard_uuid, player_id, position) in ready {
        if let Err(e) = spawn_player_on_shard(broker, shard_uuid, player_id, position, entity_map, flagged_for_rebuild).await {
            tracing::error!("Failed to spawn player player_id={} on shard shard_uuid={}: {}", player_id, shard_uuid, e);
        } else {
            tracing::info!("Successfully processed pending spawn for player_id={}", player_id);
        }
    }
}

fn handle_entity_position_update_topic(payload: &[u8], shard_map: &SharedShardMap, entity_map: &SharedEntityMap, flagged_for_rebuild: &mut bool) {
    let Some(parsed) = deserialize_position_payload(payload) else {
        return;
    };

    tracing::debug!(
        "Received position update for entity_id={} at ({}, {})",
        parsed.connection_id, parsed.position[0], parsed.position[1]
    );

    //check if the entity has moved into a different shard
    let entity_map = entity_map.write().unwrap();
    let old_position = entity_map.get(&parsed.connection_id).cloned();
    
    let old_shard = old_position.and_then(|pos| find_shard_for_position(shard_map, pos[0], pos[1]));
    let new_shard = find_shard_for_position(shard_map, parsed.position[0], parsed.position[1]);

    if old_shard.map(|(b, _)| b) != new_shard.map(|(b, _)| b) {
        tracing::info!(
            "Entity entity_id={} moved from shard {:?} to shard {:?}",
            parsed.connection_id,
            old_shard.and_then(|(_, uuid)| uuid),
            new_shard.and_then(|(_, uuid)| uuid)
        );
        *flagged_for_rebuild = true;
    }
}