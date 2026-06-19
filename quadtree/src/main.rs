mod quic_client;
mod config;

use common::{Boundary, Quadrant};
use common::topics::{
    PositionPayload, QuadtreeBoundariesUpdatePayload, StartingPositionPayload, Topic, ReleaseOwnershipPayload, deserialize_position_payload, deserialize_shard_created_payload, deserialize_starting_position_payload, serialize_position_payload, serialize_quadtree_boundaries_update_payload, serialize_release_ownership_payload
};
use common::BrokerMessage;
use game_sockets::GameNetworkEvent;
use quic_client::QuicClient;
use std::collections::{HashMap, HashSet, VecDeque};
use std::mem;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use crate::config::Config;

type SharedShardSet = Arc<RwLock<HashSet<Boundary>>>;
type SharedShardMap = Arc<RwLock<HashMap<Boundary, Option<uuid::Uuid>>>>;
type SharedPendingPlayers = Arc<tokio::sync::Mutex<VecDeque<(Instant, StartingPositionPayload)>>>;
type SharedEntityMap = Arc<RwLock<HashMap<uuid::Uuid, EntityData>>>;
type SharedEntityOwners = Arc<RwLock<HashMap<uuid::Uuid, uuid::Uuid>>>;
type PendingShardToDestroy = HashSet<(uuid::Uuid, Boundary)>;

const PLAYER_SPAWN_RETRY_TIMEOUT_SECS: u64 = 15;

struct EntityData {
    position: [f64; 2],
    entities_in_interest: HashSet<uuid::Uuid>,
    parent_boundary: Boundary,
    owner_boundary: Boundary,
    ghosted_boundaries: HashSet<Boundary>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();
    tracing::info!("Quadtree starting with config: world_size={}, max_capacity={}, max_depth={}, nearby_margin={}, area_of_interest_radius={}",
        config.world_size, config.max_capacity, config.max_depth, config.nearby_margin, config.area_of_interest_radius);

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
    let entity_owners: SharedEntityOwners = Arc::new(RwLock::new(HashMap::new()));

    run_main_loop(
        config,
        orchestrator_client,
        broker_client,
        shard_set,
        shard_map,
        pending_players,
        entity_map,
        entity_owners,
    )
    .await
}


async fn run_main_loop(
    config: Config,
    orchestrator_client: Option<QuicClient>,
    broker_client: Option<QuicClient>,
    shard_set: SharedShardSet,
    shard_map: SharedShardMap,
    pending_players: SharedPendingPlayers,
    entity_map: SharedEntityMap,
    entity_owners: SharedEntityOwners,
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
        &shard_set,
        &shard_map,
    );

    let orchestrator_client = orchestrator_client;
    let mut broker_client = broker_client;
    let mut pending_shard_to_destroy: PendingShardToDestroy = HashSet::new();
    let mut should_handle_pending_destructions = false;

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
    let mut tick = tokio::time::interval(Duration::from_millis(config.quadtree_tick_ms));

    loop {
        tick.tick().await;

        let mut flagged_for_rebuild = false;
        
        let old_shard_map = shard_map.read().unwrap().clone();
        

        if let Some(client) = broker_client.as_mut() {
            poll_quic_events(client, "broker", &shard_map, &pending_players, &entity_map, &entity_owners, &mut flagged_for_rebuild, &mut should_handle_pending_destructions).await?;
        }

        if let Some(client) = broker_client.as_ref() {
            process_pending_players(&pending_players, client, &shard_map, &entity_map, &entity_owners, &mut flagged_for_rebuild, &shard_set).await;
        }

        if let Some(client) = broker_client.as_ref() {
            apply_area_of_interest(client, &entity_map).await;
        }
        
        if flagged_for_rebuild || should_handle_pending_destructions{
            tracing::info!("Rebuilding quadtree due to shard changes...");

            let old_shard_set = shard_set.read().unwrap().clone();
            // print the current shard map for debugging
            tracing::info!("Current shard map:");
            for (boundary, uuid) in old_shard_map.iter() {
                tracing::info!("Boundary: ({}, {}, {}), UUID: {:?}", boundary.x, boundary.y, boundary.half_size, uuid);
            }
            let points: Vec<[f64; 2]> = {
                let entity_map = entity_map.read().unwrap();
                entity_map.values().map(|data| data.position).collect()
            };

            quadtree.rebuild(boundary, points, &shard_set, &shard_map, &mut pending_shard_to_destroy).await;

            let new_shard_set = shard_set.read().unwrap().clone();
            let mut rebuilt_boundaries: Vec<Boundary> = new_shard_set.iter().copied().collect();

            if new_shard_set != old_shard_set || should_handle_pending_destructions {
                // add the older shards for each new shard which has not it's uuid assigned yet in the rebuilt_boundaries so orchestrator don't kill them before the new ones are up
                retain_old_shards_during_rebuild(&old_shard_set,&new_shard_set,&shard_set,&old_shard_map,&mut rebuilt_boundaries, &mut pending_shard_to_destroy,).await;
                tracing::info!("Pending to destroy: {:?}", pending_shard_to_destroy);
                if let Some(broker) = broker_client.as_ref() {
                    handle_pending_shard_destructions(broker, &entity_map, &shard_map, &entity_owners, &shard_set,&mut pending_shard_to_destroy).await;
                }
                
                tracing::info!("Shard set changed after rebuild, sending updated configuration to orchestrator...");
                if let Some(client) = orchestrator_client.as_ref() {       
                    send_server_configuration_update(client, rebuilt_boundaries.clone()).await?;
                    //stage_pending_shard_spawns(&pending_shard_spawns, &entity_map, rebuilt_boundaries).await;
                } else {
                    tracing::warn!("No connection to orchestrator, skipping shard configuration update");
                }
                let new_boundaries: Vec<Boundary> = shard_set.read().unwrap().iter().copied().collect();
                    //broadcast the new shard boundaries to all connected clients
                let payload = serialize_quadtree_boundaries_update_payload(&QuadtreeBoundariesUpdatePayload {
                    margin: config.nearby_margin as f32,
                    boundaries: new_boundaries.clone(),
                });
                if let Some(broker) = broker_client.as_ref() {
                    broker.publish(Topic::QuadtreeBoundariesUpdate, &payload).await?;
                }

                // print should_handle_pending_destructions for debugging
                let number_of_pending_destructions = pending_shard_to_destroy.len();
                should_handle_pending_destructions = number_of_pending_destructions > 0;
                //tracing::info!("should_handle_pending_destructions={}", should_handle_pending_destructions);
            }
        }
    }
}

async fn poll_quic_events(
    broker: &mut QuicClient,
    label: &str,
    shard_map: &SharedShardMap,
    pending_players: &SharedPendingPlayers,
    entity_map: &SharedEntityMap,
    entity_owners: &SharedEntityOwners,
    flagged_for_rebuild: &mut bool,
    should_handle_pending_destructions: &mut bool,
) -> anyhow::Result<()> {
    while let Some(event) = broker.poll()? {
        match event {
            GameNetworkEvent::Message { data, connection, stream } => {
                tracing::debug!(
                    "{} link message: {} bytes from {:?} on stream {}",
                    label,
                    data.len(),
                    connection.connection_id,
                    stream.stream_id
                );

                handle_quic_message(&data, shard_map, pending_players, entity_map, entity_owners, flagged_for_rebuild, broker, should_handle_pending_destructions).await;
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
}

impl Quadtree {
    fn new(
        boundary: Boundary,
        depth: u8,
        max_depth: u8,
        max_capacity: usize,
    ) -> Self {
        Self {
            boundary,
            points: Vec::new(),
            depth,
            max_depth,
            max_capacity,
            children: None,
        }
    }

    fn new_root(
        boundary: Boundary,
        max_depth: u8,
        max_capacity: usize,
        shard_set: &SharedShardSet,
        shard_map: &SharedShardMap,
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

        Self::new(boundary, 0, max_depth, max_capacity)
    }

    fn insert(&mut self, point: [f64; 2], shard_set: &SharedShardSet, shard_map: &SharedShardMap) {
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

            self.split(shard_set, shard_map);
        }

        self.insert_into_child(point, shard_set, shard_map);
    }

    fn split(&mut self, shard_set: &SharedShardSet, shard_map: &SharedShardMap) {
        let boundaries = self.boundary.subdivide();

        // REMOVE the shard_set and shard_map write locks from here!
        // They cause premature state updates during incremental insertions.

        let mut children = [
            Box::new(Quadtree::new(boundaries[0], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[1], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[2], self.depth + 1, self.max_depth, self.max_capacity)),
            Box::new(Quadtree::new(boundaries[3], self.depth + 1, self.max_depth, self.max_capacity)),
        ];

        let old = mem::take(&mut self.points);
        for point in old {
            let idx = match self.boundary.quadrant(&point[0], &point[1]) {
                Quadrant::NorthEast => 0,
                Quadrant::NorthWest => 1,
                Quadrant::SouthEast => 2,
                Quadrant::SouthWest => 3,
            };
            children[idx].insert(point, shard_set, shard_map);
        }

        self.children = Some(children);
    }

    fn insert_into_child(&mut self, point: [f64; 2], shard_set: &SharedShardSet, shard_map: &SharedShardMap) {
        let idx = match self.boundary.quadrant(&point[0], &point[1]) {
            Quadrant::NorthEast => 0,
            Quadrant::NorthWest => 1,
            Quadrant::SouthEast => 2,
            Quadrant::SouthWest => 3,
        };

        self.children.as_mut().unwrap()[idx].insert(point, shard_set, shard_map);
    }

    async fn rebuild(&mut self, boundary: Boundary, points: Vec<[f64; 2]>, shard_set: &SharedShardSet, shard_map: &SharedShardMap, pending_shard_to_destroy: &mut PendingShardToDestroy) {
        // 1. Reset structure
        self.boundary = boundary;
        self.points.clear();
        self.children = None;

        // 2. Build the entire tree structure recursively
        for point in points {
            self.insert(point, shard_set, shard_map);
        }

        // 3. Collect the resulting leaf boundaries
        let mut final_leaves = Vec::new();
        self.collect_leaf_boundaries(&mut final_leaves);

        // 4. Update shard_set all at once
        {
            let mut set = shard_set.write().unwrap();
            set.clear();
            for leaf in &final_leaves {
                set.insert(*leaf);
            }
        }

        // 5. Sync shard_map without wiping out existing UUID connections
        {
            let mut map = shard_map.write().unwrap();
            let old_map = mem::take(&mut *map);

            for leaf in final_leaves {
                // Retain the existing Shard UUID connection if we already have it
                let mut existing_uuid = old_map.get(&leaf).and_then(|id| *id);
                let mut shard_to_remove_from_destroy: Option<(uuid::Uuid, Boundary)> = None;
                for pending_shard in pending_shard_to_destroy.iter() {
                    if pending_shard.1.x == leaf.x && pending_shard.1.y == leaf.y && pending_shard.1.half_size == leaf.half_size {
                        tracing::info!("Retaining old shard {:?} in shard_map during rebuild to avoid pending destruction", leaf);
                        existing_uuid = Some(pending_shard.0);
                        shard_to_remove_from_destroy = Some((*pending_shard).clone());
                        break;
                    }
                }
                if let Some(shard_to_remove) = shard_to_remove_from_destroy {
                    pending_shard_to_destroy.remove(&shard_to_remove);
                }
                map.insert(leaf, existing_uuid);
            }
        }
    }

    // Recursively find all leaf nodes (nodes with no children)
    fn collect_leaf_boundaries(&self, vec: &mut Vec<Boundary>) {
        if let Some(ref children) = self.children {
            for child in children {
                child.collect_leaf_boundaries(vec);
            }
        } else {
            vec.push(self.boundary);
        }
    }
}

async fn send_server_configuration_update(client: &QuicClient, boundaries: Vec<Boundary>) -> anyhow::Result<()> {
    client.send_shard_data(&boundaries).await?;
    Ok(())
}

async fn handle_quic_message(
    data: &[u8],
    shard_map: &SharedShardMap,
    pending_players: &SharedPendingPlayers,
    entity_map: &SharedEntityMap,
    entity_owners: &SharedEntityOwners,
    flagged_for_rebuild: &mut bool,
    broker: &QuicClient,
    should_handle_pending_destructions: &mut bool,
) {
    let Some(message) = BrokerMessage::deserialize(data) else {
        return;
    };

    match message {
        BrokerMessage::Broadcast { topic, payload }
        | BrokerMessage::Publish { topic, payload } => {
            match Topic::from_bytes(topic) {
                Topic::PlayerStartingPosition => {
                    handle_player_starting_position_topic(&payload, pending_players);
                }
                Topic::ShardCreated => handle_shard_created_topic(&payload, shard_map, flagged_for_rebuild, entity_map, broker, should_handle_pending_destructions).await,
                Topic::EntityPositionUpdate(uuid) => {
                    handle_entity_position_update_topic(uuid, &payload, shard_map, entity_map, entity_owners, flagged_for_rebuild, broker).await
                }
                Topic::Disconnect(uuid) => {
                    tracing::info!("Received disconnect for entity_id={}", uuid);
                    entity_map.write().unwrap().remove(&uuid);
                    entity_owners.write().unwrap().remove(&uuid);
                    if let Err(e) = broker.unsubscribe(uuid, Topic::EntityPositionUpdate(uuid)).await {
                        tracing::error!("Failed to unsubscribe from position updates for entity {:?}: {}", uuid, e);
                    } else {
                        tracing::info!("Unsubscribed from position updates for entity {:?} due to disconnect", uuid);
                    }
                    *flagged_for_rebuild = true;
                }
                _ => {}
            }
        }
        _ => {}
    }
}

async fn  handle_shard_created_topic(
    payload: &[u8],
    shard_map: &SharedShardMap,
    flagged_for_rebuild: &mut bool,
    entity_map: &SharedEntityMap,
    broker: &QuicClient,
    should_handle_pending_destructions: &mut bool,
) {
    let Some(parsed) = deserialize_shard_created_payload(payload) else {
        tracing::error!("Failed to deserialize ShardCreated payload") ;
        return;
    };
    *flagged_for_rebuild = true;
    *should_handle_pending_destructions = true;
        let mut map = shard_map.write().unwrap();
        map.insert(parsed.boundary, Some(parsed.shard_connection_id));
        tracing::info!(
            "Shard registered: shard_uuid={} boundary=({}, {}, {})",
            parsed.shard_connection_id, parsed.boundary.x, parsed.boundary.y, parsed.boundary.half_size
        );
        // for entities in new shard, subscribe the new shard to their position updates
        for (entity_id, data) in entity_map.read().unwrap().iter() {
            if parsed.boundary.contains(&data.position[0], &data.position[1]) {
                tracing::info!("Subscribing new shard shard_uuid={} to position updates for entity_id={} due to shard creation", parsed.shard_connection_id, entity_id);
                let topic = Topic::EntityPositionUpdate(*entity_id);
                broker.subscribe(parsed.shard_connection_id, topic).await
                .unwrap_or_else(|e| tracing::error!("Failed to subscribe new shard to entity position updates: {}", e));
                let payload = serialize_position_payload(&PositionPayload {
                    position: data.position,
                });
                broker.publish(Topic::EntityPositionUpdate(*entity_id), &payload).await.unwrap_or_else(|e| tracing::error!("Failed to publish entity position to new shard: {}", e));
            }
        }
}

fn handle_player_starting_position_topic(payload: &[u8], pending_players: &SharedPendingPlayers) {
    if let Some(parsed) = deserialize_starting_position_payload(payload) {
        handle_player_starting_position_payload(parsed, pending_players);
    }
}

fn handle_player_starting_position_payload(
    payload: StartingPositionPayload,
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
    entity_owners: &SharedEntityOwners,
    flagged_for_rebuild: &mut bool,
    shard_map: &SharedShardMap,
    shard_set: SharedShardSet
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
    broker.subscribe(player_id, Topic::CastAbility(player_id)).await?;

    //send the initial position update so the client and quadtree have a baseline position for the player
     let payload = serialize_position_payload(&PositionPayload {
        position,
    });

    broker.publish(Topic::EntityPositionUpdate(player_id), &payload).await?;   

    //subscribe the quadtree to the player's position updates so it can track which shard they are in
    broker.subscribe(broker.connection_id(), Topic::EntityPositionUpdate(player_id)).await?; 
    broker.subscribe(broker.connection_id(), Topic::Disconnect(player_id)).await?;

    let boundary = find_shard_for_position(shard_map, position[0], position[1])
        .map(|(b, _)| b)
        .unwrap_or_else(|| Boundary { x: 0.0, y: 0.0, half_size: 100.0 });

    let entity_data = EntityData {
        position,
        entities_in_interest: HashSet::new(),
        parent_boundary: boundary,
        owner_boundary: boundary,
        ghosted_boundaries: HashSet::new(),
    };

    entity_map.write().unwrap().insert(player_id, entity_data);
    entity_owners.write().unwrap().insert(player_id, shard_uuid);

    let config = Config::from_env();

    if entity_map.read().unwrap().len() >= config.max_capacity {
        *flagged_for_rebuild = true;
    }

    // Publish the starting position so the shard server spawns the player.
    let payload_bytes = serialize_position_payload(&PositionPayload { position });
    broker.publish(Topic::PlayerStartingPositionInShard(player_id), &payload_bytes).await?;

    // send the quadtree boundaries to new client for debugging and visualization purposes
    let new_boundaries: Vec<Boundary> = shard_set.read().unwrap().iter().copied().collect();
                    //broadcast the new shard boundaries to all connected clients
            let payload = serialize_quadtree_boundaries_update_payload(&QuadtreeBoundariesUpdatePayload {
                margin: config.nearby_margin as f32,
                boundaries: new_boundaries.clone(),
            });
    broker.publish(Topic::QuadtreeBoundariesUpdate, &payload).await?;

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
    entity_owners: &SharedEntityOwners,
    flagged_for_rebuild: &mut bool,
    shard_set: &SharedShardSet
) {
    let timeout = Duration::from_secs(PLAYER_SPAWN_RETRY_TIMEOUT_SECS);
    let mut ready: Vec<(uuid::Uuid, uuid::Uuid, [f64; 2])> = Vec::new();
    let mut still_pending: VecDeque<(Instant, StartingPositionPayload)> = VecDeque::new();

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
        if let Err(e) = spawn_player_on_shard(broker, shard_uuid, player_id, position, entity_map, entity_owners, flagged_for_rebuild, shard_map, shard_set.clone()).await {
            tracing::error!("Failed to spawn player player_id={} on shard shard_uuid={}: {}", player_id, shard_uuid, e);
        } else {
            tracing::info!("Successfully processed pending spawn for player_id={}", player_id);
        }
    }
}

async fn handle_entity_position_update_topic(connection_id: uuid::Uuid, payload: &[u8], shard_map: &SharedShardMap, entity_map: &SharedEntityMap, entity_owners: &SharedEntityOwners, flagged_for_rebuild: &mut bool, broker: &QuicClient) {
    let Some(parsed) = deserialize_position_payload(payload) else {
        print!("Failed to deserialize EntityPositionUpdate payload") ;
        return;
    };
    let is_known_entity = entity_map.read().unwrap().contains_key(&connection_id);
    if !is_known_entity {
        tracing::warn!("Received position update for unknown entity_id={}, treating as new spawn at ({}, {})", connection_id, parsed.position[0], parsed.position[1]);
        return;
    }

    // Phase 1: collect all needed data under short-lived read locks, then release them
    // before any await points to avoid holding locks across suspension points.
    let (entities_in_interest, ghosted_boundaries, old_shard, new_shard) = {
        let map = entity_map.read().unwrap();
        let entity = map.get(&connection_id);
        let entities_in_interest = entity
            .map(|data| data.entities_in_interest.clone())
            .unwrap_or_default();
        let ghosted_boundaries = entity
            .map(|data| data.ghosted_boundaries.clone())
            .unwrap_or_default();
        let old_position = entity.map(|data| data.position);
        drop(map); // release read lock before acquiring shard_map read lock in find_shard_for_position

        let old_shard = old_position.and_then(|pos| find_shard_for_position(shard_map, pos[0], pos[1]));
        let new_shard = find_shard_for_position(shard_map, parsed.position[0], parsed.position[1]);
        (entities_in_interest, ghosted_boundaries, old_shard, new_shard)
    };

    tracing::debug!(
        "Received position update for entity_id={} at ({}, {})",
        connection_id, parsed.position[0], parsed.position[1]
    );

    if old_shard.map(|(b, _)| b) != new_shard.map(|(b, _)| b) {
        tracing::info!(
            "Entity entity_id={} moved from shard {:?} to shard {:?}",
            connection_id,
            old_shard.and_then(|(_, uuid)| uuid),
            new_shard.and_then(|(_, uuid)| uuid)
        );
        *flagged_for_rebuild = true;
    }


    // Phase 2: async ghosting check with no locks held (reads entity_map internally).
    check_for_shard_ghosting(
        broker,
        connection_id,
        entity_map,
        shard_map,
        entity_owners,
    ).await;

    // Phase 3: perform the async handoff check with NO locks held.

    // 1. Get the owner of connection_id safely in a scoped block
    let owner_id = {
        let owners = entity_owners.read().unwrap();
        owners.get(&connection_id).copied()
    };
    let owner_boundary = {
        if let Some(owner_id) = owner_id {
            shard_map.read().unwrap().iter().find_map(|(boundary, maybe_uuid)| {
                if maybe_uuid.map(|uuid| uuid == owner_id).unwrap_or(false) {
                    Some(*boundary)
                } else {
                    None
                }
            })
        } else {
            None
        }
    };

    // 2. Get the boundaries of the owner_id (shard) safely in a scoped block
    let new_parent_boundary = {
        if let Some((new_boundary, _)) = new_shard {
            new_boundary
        } else if let Some((old_boundary, _)) = old_shard {
            old_boundary
        } else {
            Boundary { x: 0.0, y: 0.0, half_size: 100.0 }
        }
    };
    
    let new_owner_boundary = if let Some(current_owner) = owner_boundary {
        let was_handoff = check_for_handoff(
            broker,
            parsed.position,
            connection_id,
            current_owner,
            new_parent_boundary,
            shard_map,
            entity_owners,
        ).await;

        if was_handoff { new_parent_boundary } else { current_owner }
    } else {
        new_parent_boundary
    };
    
    // Phase 4: acquire the write lock briefly, insert, then release immediately.
    {
        let mut map = entity_map.write().unwrap();
        if let Some(entity_data) = map.get_mut(&connection_id) {
            entity_data.position = parsed.position;
            entity_data.entities_in_interest = entities_in_interest;
            entity_data.parent_boundary = new_parent_boundary;
            entity_data.owner_boundary = new_owner_boundary;
            // Don't update ghosted_boundaries here since check_for_shard_ghosting will handle that separately
        } else {
            map.insert(connection_id, EntityData {
                position: parsed.position,
                entities_in_interest,
                parent_boundary: new_parent_boundary,
                ghosted_boundaries,
                owner_boundary: new_owner_boundary,
            });
        }
    }

}

async fn apply_area_of_interest(broker: &QuicClient, entity_map: &SharedEntityMap) {
// 1. Calculate all the changes while holding the READ lock
    let mut pending_updates = Vec::new();
    
    {
        let entity_map_reader = entity_map.read().unwrap();
        for (entity_id, data) in entity_map_reader.iter() {
            let mut nearby_entities = HashSet::new();
            
            for (other_id, other_data) in entity_map_reader.iter() {
                if entity_id == other_id { continue; }
                
                let dx = data.position[0] - other_data.position[0];
                let dy = data.position[1] - other_data.position[1];
                let distance_squared = dx * dx + dy * dy;
                let radius_squared = Config::from_env().area_of_interest_radius * Config::from_env().area_of_interest_radius;
                
                if distance_squared <= radius_squared {
                    nearby_entities.insert(*other_id);
                }
            }

            tracing::debug!(
                "Entity {:?} has {} nearby entities within area of interest",
                entity_id,
                nearby_entities.len()
            );
            
            let new_interests: HashSet<uuid::Uuid> = nearby_entities.difference(&data.entities_in_interest).cloned().collect();
            let no_longer_in_interest: HashSet<uuid::Uuid> = data.entities_in_interest.difference(&nearby_entities).cloned().collect();
            
            if !new_interests.is_empty() || !no_longer_in_interest.is_empty() {
                // Save the intended state changes to apply AFTER dropping the read lock
                pending_updates.push((*entity_id, nearby_entities, new_interests, no_longer_in_interest));
            }
        }
    } // READ LOCK DROPS HERE

    // 2. Apply the state changes holding the WRITE lock
    {
        let mut entity_map_writer = entity_map.write().unwrap();
        for (entity_id, nearby_entities, _, _) in &pending_updates {
            if let Some(data) = entity_map_writer.get_mut(entity_id) {
                data.entities_in_interest = nearby_entities.clone();
            }
        }
    } // WRITE LOCK DROPS HERE

    for (entity_id, _, new_interests, no_longer_in_interest) in pending_updates {
        for new_id in new_interests {
            // Send subscription message for new_id to the client
            if let Err(e) = broker.subscribe(entity_id, Topic::EntityPositionUpdate(new_id)).await {
                tracing::error!("Failed to subscribe to position updates for entity {:?}: {}", new_id, e);
            } else {
                tracing::info!("Subscribed to position updates for entity {:?} as it entered the area of interest of entity {:?}", new_id, entity_id);
            }
            // send subscription to disconnect topic as well
            if let Err(e) = broker.subscribe(entity_id, Topic::Disconnect(new_id)).await {
                tracing::error!("Failed to subscribe to disconnect updates for entity {:?}: {}", new_id, e);
            } else {
                tracing::info!("Subscribed to disconnect updates for entity {:?} as it entered the area of interest of entity {:?}", new_id, entity_id);
            }

            // Send subscription to CastAbility topic
            if let Err(e) = broker.subscribe(entity_id, Topic::CastAbility(new_id)).await {
                tracing::error!("Failed to subscribe to cast ability updates for entity {:?}: {}", new_id, e);
            } else {
                tracing::info!("Subscribed to cast ability updates for entity {:?} as it entered the area of interest of entity {:?}", new_id, entity_id);
            }
        }

        for old_id in no_longer_in_interest {
            // Send unsubscription message for old_id to the client
            if let Err(e) = broker.unsubscribe(entity_id, Topic::EntityPositionUpdate(old_id)).await {
                tracing::error!("Failed to unsubscribe from position updates for entity {:?}: {}", old_id, e);
            } else {
                tracing::info!("Unsubscribed from position updates for entity {:?} as it left the area of interest of entity {:?}", old_id, entity_id);
            }
            // send unsubscription to disconnect topic as well
            if let Err(e) = broker.unsubscribe(entity_id, Topic::Disconnect(old_id)).await {
                tracing::error!("Failed to unsubscribe from disconnect updates for entity {:?}: {}", old_id, e);
            } else {
                tracing::info!("Unsubscribed from disconnect updates for entity {:?} as it left the area of interest of entity {:?}", old_id, entity_id);
            }

            // Send unsubscription to CastAbility topic
            if let Err(e) = broker.unsubscribe(entity_id, Topic::CastAbility(old_id)).await {
                tracing::error!("Failed to unsubscribe from cast ability updates for entity {:?}: {}", old_id, e);
            } else {
                tracing::info!("Unsubscribed from cast ability updates for entity {:?} as it left the area of interest of entity {:?}", old_id, entity_id);
            }
        }
    }
}

async fn check_for_handoff(
    broker: &QuicClient,
    position: [f64; 2],
    entity_id: uuid::Uuid,
    old_shard: Boundary,
    new_shard: Boundary,
    shard_map: &SharedShardMap,
    entity_owners: &SharedEntityOwners,
) -> bool {
    //if the entity is no longuer whithin the margins of its old shard, swap the input subscription to the new shard and have the new shard claim ownership of the entity
    if !is_within_margin(&old_shard, position[0], position[1], Config::from_env().nearby_margin) {
        /*println!("HANDOFF: Entity entity_id={} is outside the margin of its old shard boundary=({}, {}, {}), initiating handoff to new shard boundary=({}, {}, {})",
            entity_id,
            old_shard.x, old_shard.y, old_shard.half_size,
            new_shard.x, new_shard.y, new_shard.half_size
        );*/
        if let Some(new_shard_uuid) = shard_map.read().unwrap().get(&new_shard).and_then(|uuid| *uuid) {
            //println!("HANDOFF: Found new shard UUID {:?} for new shard boundary=({}, {}, {})", new_shard_uuid, new_shard.x, new_shard.y, new_shard.half_size);
            //unsubscribe the old shard from the player's input and disconnect topics
            if let Some(old_shard_uuid) = shard_map.read().unwrap().get(&old_shard).and_then(|uuid| *uuid) {
                let payload = serialize_release_ownership_payload(&ReleaseOwnershipPayload {
                    entity_id,
                    shard_id: new_shard_uuid,
                });
                broker.publish(Topic::ReleaseOwnership(old_shard_uuid), &payload).await.ok();
            }
            else { // Release ownership will send the claim otherwise to ensure one authority at a time
                broker.publish(Topic::ClaimOwnership(new_shard_uuid), entity_id.as_bytes()).await.ok();
            }
            //claim sent to new shard by old shard

            entity_owners.write().unwrap().insert(entity_id, new_shard_uuid);

            tracing::info!(
                "Handoff: Entity entity_id={} moved from shard {:?} to shard {:?}",
                entity_id,
                old_shard,
                new_shard
            );

            return true;
        }
    }

    false
}

async fn check_for_shard_ghosting(
    broker: &QuicClient,
    entity_id: uuid::Uuid,
    entity_map: &SharedEntityMap,
    shard_map: &SharedShardMap,
    entity_owners: &SharedEntityOwners,
) {
    // Compare desired ghost subscriptions against the previous set so shards stop simulating
    // ghosts once the entity leaves their margin.
    let nearby_margin = Config::from_env().nearby_margin;
    let (position, current_boundary, previous_ghosted_boundaries) = {
        let entity_map = entity_map.read().unwrap();
        let Some(entity) = entity_map.get(&entity_id) else {
            return;
        };
        (
            entity.position,
            entity.owner_boundary,
            entity.ghosted_boundaries.clone(),
        )
    };
    let owner_of_entity = entity_owners.read().unwrap().get(&entity_id).copied();

    let desired_ghosts = {
        let map = shard_map.read().unwrap();
        map.iter()
            .filter_map(|(boundary, maybe_uuid)| {
                if *boundary == current_boundary || !is_within_margin(boundary, position[0], position[1], nearby_margin) {
                    return None;
                }

                maybe_uuid.map(|shard_uuid| (*boundary, shard_uuid))
            })
            .collect::<HashMap<Boundary, uuid::Uuid>>()
    };

    for (boundary, shard_uuid) in &desired_ghosts {
        if previous_ghosted_boundaries.contains(boundary) || Some(*shard_uuid) == owner_of_entity {
            continue;
        }

        if let Err(e) = broker.subscribe(*shard_uuid, Topic::EntityPositionUpdate(entity_id)).await {
            tracing::error!("Failed to subscribe to shard {:?} to position updates for entity {:?}: {}", shard_uuid, entity_id, e);
        } else {
            tracing::info!("Subscribed shard {:?} to position updates for entity {:?} as it is within nearby margin of the shard boundary", shard_uuid, entity_id);
        }
    }

    let stale_ghosts = {
        let map = shard_map.read().unwrap();
        previous_ghosted_boundaries
            .iter()
            .filter_map(|boundary| {
                if desired_ghosts.contains_key(boundary) {
                    return None;
                }

                map.get(boundary)
                    .and_then(|maybe_uuid| maybe_uuid.map(|shard_uuid| (*boundary, shard_uuid)))
            })
            .collect::<Vec<_>>()
    };

    for (_, shard_uuid) in &stale_ghosts {
        if let Err(e) = broker.unsubscribe(*shard_uuid, Topic::EntityPositionUpdate(entity_id)).await {
            tracing::error!("Failed to unsubscribe shard {:?} from position updates for entity {:?}: {}", shard_uuid, entity_id, e);
        } else {
            tracing::info!("Unsubscribed shard {:?} from position updates for entity {:?} after leaving nearby margin", shard_uuid, entity_id);
        }
    }

    let updated_ghosted_boundaries = desired_ghosts.keys().copied().collect::<HashSet<_>>();
    if let Some(entity) = entity_map.write().unwrap().get_mut(&entity_id) {
        entity.ghosted_boundaries = updated_ghosted_boundaries;
    }
}

fn is_within_margin(boundary: &Boundary, x: f64, y: f64, margin: f64) -> bool {
    let left = boundary.x - boundary.half_size;
    let right = boundary.x + boundary.half_size;
    let top = boundary.y + boundary.half_size;
    let bottom = boundary.y - boundary.half_size;

    (x >= left - margin && x <= right + margin) && (y >= bottom - margin && y <= top + margin)
}

/// function to keep old shards while the new ones aren't ready
async fn retain_old_shards_during_rebuild(
    old_shard_set: &HashSet<Boundary>,
    new_shard_set: &HashSet<Boundary>,
    shard_set: &SharedShardSet,
    shard_map: &HashMap<Boundary, Option<uuid::Uuid>>,
    rebuilt_boundaries: &mut Vec<Boundary>,
    pending_shard_to_destroy: &mut PendingShardToDestroy,
) {
    for new_shard in new_shard_set {
        if let Some(_) = shard_map.get(new_shard).and_then(|id| *id) {
            continue;
        }
        else {
            let parent_shard = old_shard_set.iter().find(|old| old.contains(&new_shard.x, &new_shard.y) && old.half_size > new_shard.half_size);
                if let Some(parent) = parent_shard {
                    if parent == new_shard {
                        continue; // skip if the new shard is the same as the parent shard
                    }
                tracing::info!("keeping parent shard for new shard ({}, {}, {}) during rebuild: parent boundary=({}, {}, {})", new_shard.x, new_shard.y, new_shard.half_size, parent.x, parent.y, parent.half_size);
                rebuilt_boundaries.push(*parent);
                let parent_uuid = shard_map.get(parent).and_then(|id| *id);
                if !parent_uuid.is_some() {
                    tracing::warn!("Parent shard boundary=({}, {}, {}) for new shard ({}, {}, {}) has no registered shard UUID, not marking for destruction (maybe already done)", parent.x, parent.y, parent.half_size, new_shard.x, new_shard.y, new_shard.half_size);
                }
                else{
                    tracing::info!("Parent shard boundary=({}, {}, {}) for new shard ({}, {}, {}) has registered shard UUID {:?}, retaining during rebuild", parent.x, parent.y, parent.half_size, new_shard.x, new_shard.y, new_shard.half_size, parent_uuid);
                    pending_shard_to_destroy.insert((parent_uuid.unwrap(), *parent));
                }
                shard_set.write().unwrap().insert(*parent);
                }
                else{
                    // find all the childrens
                    let childrens = old_shard_set.iter().filter(|old| new_shard.contains(&old.x, &old.y)).copied().collect::<Vec<_>>();
                    tracing::info!("keeping children shards for new shard ({}, {}, {}) during rebuild: {} children found", new_shard.x, new_shard.y, new_shard.half_size, childrens.len());
                    rebuilt_boundaries.extend(childrens.clone());
                    for child in childrens {
                        if child.x == new_shard.x && child.y == new_shard.y && child.half_size == new_shard.half_size {
                            continue; // skip if the new shard is the same as the child shard
                        }
                        shard_set.write().unwrap().insert(child);
                        let child_uuid = shard_map.get(&child).and_then(|id| *id);
                        if !child_uuid.is_some() {
                            tracing::warn!("Child shard boundary=({}, {}, {}) for new shard ({}, {}, {}) has no registered shard UUID, not marking for destruction (may be already done)", child.x, child.y, child.half_size, new_shard.x, new_shard.y, new_shard.half_size);
                        }
                        else{
                            tracing::info!("Child shard boundary=({}, {}, {}) for new shard ({}, {}, {}) has registered shard UUID {:?}, retaining during rebuild", child.x, child.y, child.half_size, new_shard.x, new_shard.y, new_shard.half_size, child_uuid);
                            pending_shard_to_destroy.insert((child_uuid.unwrap(), child));
                        }
                    }
                }
        }
    }
}

async fn handle_shard_destruction(
    broker: &QuicClient,
    destroyed_shard_uuid: uuid::Uuid,
    destroyed_boundary: Boundary,
    entity_map: &SharedEntityMap,
    shard_map: &SharedShardMap,
    entity_owners: &SharedEntityOwners,
) {
    let margin = Config::from_env().nearby_margin;
    let entities_to_process = {
        let map = entity_map.read().unwrap();
        map.iter()
            .filter(|(_, data)| destroyed_boundary.contains(&data.position[0], &data.position[1])|| is_within_margin(&destroyed_boundary, data.position[0], data.position[1], margin))
            .map(|(id, data)| (*id, data.position))
            .collect::<Vec<_>>()
    };
    tracing::info!("Processing shard destruction for shard_uuid={} with boundary=({}, {}, {}): {} entities found within or near the destroyed shard", destroyed_shard_uuid, destroyed_boundary.x, destroyed_boundary.y, destroyed_boundary.half_size, entities_to_process.len());
    for (entity_id, position) in entities_to_process {
        if let Some((_, Some(new_shard_uuid))) = find_shard_for_position(shard_map, position[0], position[1]) {
            // Prevent attempting handoff to itself
            if new_shard_uuid == destroyed_shard_uuid {
                tracing::warn!("shard_for_position returned the destroyed shard itself for entity_id={}, skipping handoff", entity_id);
                continue;
            }
            
            let is_owned = {
                let owners = entity_owners.read().unwrap();
                owners.get(&entity_id) == Some(&destroyed_shard_uuid)
            };

            if is_owned {
                let payload = serialize_release_ownership_payload(&ReleaseOwnershipPayload {
                    entity_id,
                    shard_id: new_shard_uuid,
                });
                let _ = broker.publish(Topic::ReleaseOwnership(destroyed_shard_uuid), &payload).await;
                entity_owners.write().unwrap().insert(entity_id, new_shard_uuid);
            } else {
                let _ = broker.subscribe(new_shard_uuid, Topic::EntityPositionUpdate(entity_id)).await;
            }
        }
    }

    let ownership_updates: Vec<(uuid::Uuid, uuid::Uuid)> = {
        let owners = entity_owners.read().unwrap();
        let entity_map_r = entity_map.read().unwrap();
        
        owners.iter()
            .filter(|(_, owner_id)| **owner_id == destroyed_shard_uuid)
            .filter_map(|(entity_id, _)| {
                let position = entity_map_r.get(entity_id)?.position;
                let (_, new_shard_uuid) = find_shard_for_position(shard_map, position[0], position[1])
                    .filter(|(_, uuid)| *uuid != Some(destroyed_shard_uuid))?;
                Some((*entity_id, new_shard_uuid?))
            })
            .collect()
    };

    for (entity_id, new_shard_uuid) in &ownership_updates {
        let payload = serialize_release_ownership_payload(&ReleaseOwnershipPayload {
            entity_id: *entity_id,
            shard_id: *new_shard_uuid,
        });
        let _ = broker.publish(Topic::ReleaseOwnership(destroyed_shard_uuid), &payload).await;
    }

    {
        let mut owners = entity_owners.write().unwrap();
        for (entity_id, new_shard_uuid) in ownership_updates {
            owners.insert(entity_id, new_shard_uuid);
        }
    }
}

async fn handle_pending_shard_destructions(
    broker: &QuicClient,
    entity_map: &SharedEntityMap,
    shard_map: &SharedShardMap,
    entity_owners: &SharedEntityOwners,
    shard_set: &SharedShardSet,
    pending_shard_to_destroy: &mut PendingShardToDestroy,
) {
    let mut shard_to_remove_from_pending = HashSet::<(uuid::Uuid, Boundary)>::new();
    for shard_to_destroy in pending_shard_to_destroy.iter() {
        let shard_uuid = shard_to_destroy.0;
        let shard_boundary = shard_to_destroy.1;
        let shard_in_set = shard_set.read().unwrap().contains(&shard_boundary);
        if !shard_in_set {
            handle_shard_destruction(broker, shard_uuid, shard_boundary, entity_map, shard_map, entity_owners).await;
            shard_to_remove_from_pending.insert((shard_uuid, shard_boundary));
        }
    }
    for (uuid, boundary) in shard_to_remove_from_pending {
        pending_shard_to_destroy.remove(&(uuid, boundary));
    }
}