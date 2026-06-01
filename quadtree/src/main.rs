mod quic_client;

use common::broker_messages::BrokerMessage;
use common::packets::{PositionBatch, SnapshotAuthority};
    use common::topics::{
    deserialize_starting_position_payload, deserialize_shard_created_payload,
    deserialize_shard_snapshot_payload, PositionPayload, ShardCreatedPayload, Topic,
    CrossingAlertPayload, serialize_crossing_alert_payload,
        serialize_handoff_complete_payload, HandoffCompletePayload, HandoffResult,
        serialize_forced_position_update_payload,
};
use common::{Boundary, Quadrant, ShardData, Vec2};
use game_sockets::GameNetworkEvent;
use quic_client::QuicClient;
use wincode::config;
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

    subscribe_entity_disconnect(broker_client, shard_uuid, entity_id).await?;

    Ok(())
}

async fn subscribe_entity_position_updates(
    broker_client: &QuicClient,
    shard_uuid: Uuid,
    entity_id: Uuid,
) -> anyhow::Result<()> {
    broker_client
        .subscribe(shard_uuid, Topic::ForcedPositionUpdate(entity_id))
        .await?;

    Ok(())
}

async fn subscribe_entity_disconnect(
    broker_client: &QuicClient,
    listener_uuid: Uuid,
    entity_id: Uuid,
) -> anyhow::Result<()> {
    broker_client
        .subscribe(listener_uuid, Topic::Disconnect(entity_id))
        .await?;

    Ok(())
}

async fn handle_starting_position_payload(
    broker_client: Option<&QuicClient>,
    payload: PositionPayload,
    quadtree: &Quadtree,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &HashMap<u32, Uuid>,
    nearby_margin: f64,
) -> anyhow::Result<()> {
    entity_positions.insert(payload.entity_id, payload.position);

    let Some(shard_id) = quadtree.shard_for(payload.position) else {
        return Ok(());
    };

    entity_shard_ids.insert(payload.entity_id, shard_id);

    /*
    // Boundary check for CrossingAlert 
    let nearby_shards = quadtree.shards_near(payload.position, nearby_margin);
    for near_id in nearby_shards {
        if near_id != shard_id {
            if let Some(target_uuid) = shard_uuid_by_id.get(&near_id) {
                if let Some(source_uuid) = shard_uuid_by_id.get(&shard_id) {
                    let alert = CrossingAlertPayload {
                        entity_id: entity_id_from_uuid(payload.entity_id),
                        target_shard_id: near_id,
                        target_shard_uuid: *target_uuid,
                    };
                    if let Some(client) = broker_client {
                        let _ = client.publish(
                            Topic::CrossingAlert(*source_uuid),
                            &serialize_crossing_alert_payload(&alert)
                        ).await;
                    }
                }
            }
        }
    }
    */
    if let Some(client) = broker_client {
        subscribe_entity_disconnect(client, *QUADTREE_ID, payload.entity_id).await?;

        if let Some(shard_uuid) = shard_uuid_by_id.get(&shard_id) {
            subscribe_entity_input(client, *shard_uuid, payload.entity_id).await?;
            subscribe_entity_position_updates(client, *shard_uuid, payload.entity_id).await?;
            
            if let Some(client) = broker_client {
                let _ = client.publish(
                    Topic::ForcedPositionUpdate(payload.entity_id),
                    &serialize_forced_position_update_payload(&payload)
                ).await;
            }

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
   /* 
   let Some(shard_id) = quadtree.shard_for(payload.center) else {
        return Ok(());
    };
    */
    let quadtree_shard_set: HashSet<u32> = quadtree
        .collect_shards()
        .iter()
        .filter_map(|s| s.shard_id)
        .collect();

    let currently_known_shard_ids: HashSet<u32> = shard_uuid_by_id.keys().copied().collect();

    //select a shard ID that exists in the quadtree but is not yet known by the quadtree service
    let shard_id = match quadtree_shard_set.difference(&currently_known_shard_ids).next() {
        Some(id) => *id,
        None => {
            tracing::warn!("Received ShardCreated for shard ID {} but no unknown shard IDs are available in the quadtree", payload.shard_id);
            return Ok(());
        }
    };

    //print the quadtree set
    print!("Quadtree shards: ");
    for shard_id in &quadtree_shard_set {
        print!("{} ", shard_id);
    }
    println!();

    //print the currently known shard IDs
    print!("Currently known shard IDs: ");
    for shard_id in &currently_known_shard_ids {
        print!("{} ", shard_id);
    }
    println!();

    let center: Vec2 = quadtree.shard_center(shard_id).unwrap_or(Vec2 { x: 0.0, y: 0.0 });

    shard_uuid_by_id.insert(shard_id, payload.shard_id);
    print!("Shard {} created with ID {:?} at center ({:.2}, {:.2})\n", shard_id, payload.shard_id, center.x, center.y);

    let config  = Config::from_env();

    let close_shards = quadtree.shards_near(center, config.nearby_margin);

    //subscribe the shard to the ghostupdates of its close shards to receive updates about nearby entities that may be relevant for crossing alerts
    for close_id in close_shards {
        if close_id != shard_id {
            if let Some(close_uuid) = shard_uuid_by_id.get(&close_id) {
                if let Some(client) = broker_client {
                    client.subscribe(payload.shard_id, Topic::ShardSnapshot(*close_uuid)).await?;
                }
            }
        }
    }

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
            subscribe_entity_position_updates(client, payload.shard_id, entity_id).await?;

            let position_payload: PositionPayload = PositionPayload {
                entity_id,
                position: *entity_positions.get(&entity_id).unwrap(),
            };

            if let Some(client) = broker_client {
                    let _ = client.publish(
                        Topic::ForcedPositionUpdate(position_payload.entity_id),
                        &serialize_forced_position_update_payload(&position_payload)
                    ).await;
            }
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
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
    ghost_entity_ids: &mut HashMap<Uuid, HashSet<Uuid>>,
    ghost_entity_owners: &mut HashMap<Uuid, Uuid>,
    broker_client: Option<&QuicClient>,
) -> anyhow::Result<()> {
    let config = Config::from_env();
    let snapshot = decode_shard_snapshot_payload(&payload)?;
    let batch = decode_position_batch(&snapshot)?;
    let entity_uuid_by_id = build_entity_uuid_by_id(entity_positions);
    let old_entity_positions = entity_positions.clone();

    let ghost_positions = apply_snapshot_batch(
        batch,
        entity_positions,
        entity_shard_ids,
        shard_uuid_by_id,
        ghost_entity_owners,
        &entity_uuid_by_id,
    );

    refresh_quadtree_from_snapshot(
        quadtree,
        boundary,
        max_depth,
        max_capacity,
        entity_positions,
        shard_uuid_by_id,
    );

    synchronize_non_ghost_entities(
        broker_client,
        quadtree,
        entity_positions,
        &old_entity_positions,
        entity_shard_ids,
        shard_uuid_by_id,
        ghost_entity_ids,
        config.nearby_margin,
    )
    .await?;

    process_ghost_entities(
        broker_client,
        quadtree,
        shard_uuid_by_id,
        ghost_entity_ids,
        ghost_entity_owners,
        &ghost_positions,
        &entity_uuid_by_id,
        &snapshot,
        config.nearby_margin,
    )
    .await?;

    Ok(())
}

fn decode_shard_snapshot_payload(
    payload: &[u8],
) -> anyhow::Result<common::topics::ShardSnapshotPayload> {
    deserialize_shard_snapshot_payload(payload)
        .ok_or_else(|| anyhow::anyhow!("Failed to decode shard snapshot payload"))
}

fn decode_position_batch(snapshot: &common::topics::ShardSnapshotPayload) -> anyhow::Result<PositionBatch> {
    wincode::deserialize::<PositionBatch>(&snapshot.replication)
        .map_err(|_| anyhow::anyhow!("Failed to decode shard snapshot replication batch"))
}

fn build_entity_uuid_by_id(entity_positions: &HashMap<Uuid, Vec2>) -> HashMap<u32, Uuid> {
    entity_positions
        .keys()
        .copied()
        .map(|uuid| (entity_id_from_uuid(uuid), uuid))
        .collect()
}

fn apply_snapshot_batch(
    batch: PositionBatch,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &HashMap<u32, Uuid>,
    ghost_entity_owners: &mut HashMap<Uuid, Uuid>,
    entity_uuid_by_id: &HashMap<u32, Uuid>,
) -> HashMap<u32, Vec2> {
    let mut ghost_positions = HashMap::new();

    for snap in batch.snapshots {
        let position = Vec2 {
            x: snap.x as f64,
            y: snap.y as f64,
        };

        if matches!(snap.authority, SnapshotAuthority::PendingHandOff) {
            ghost_positions.insert(snap.entity_id, position);

            if let Some(entity_uuid) = entity_uuid_by_id.get(&snap.entity_id).copied() {
                if let Some(authority_shard_id) = entity_shard_ids.get(&entity_uuid) {
                    if let Some(authority_shard_uuid) = shard_uuid_by_id.get(authority_shard_id) {
                        ghost_entity_owners.entry(entity_uuid).or_insert(*authority_shard_uuid);
                    }
                }
            }

            continue;
        }
        else if matches!(snap.authority, SnapshotAuthority::Ghost) {
            // non reliable ghost position since not owned
            continue;
        }

        if let Some(entity_uuid) = entity_uuid_by_id.get(&snap.entity_id).copied() {
            entity_positions.insert(entity_uuid, position); //non ghost since there is a continue
        }
    }

    ghost_positions
}

fn refresh_quadtree_from_snapshot(
    quadtree: &mut Quadtree,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    entity_positions: &HashMap<Uuid, Vec2>,
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
) {
    let quadtree_shard_set_before: HashSet<u32> = quadtree
        .collect_shards()
        .iter()
        .filter_map(|s| s.shard_id)
        .collect();

    let quadtree_size_before = quadtree_shard_set_before.len();

    *quadtree = rebuild_quadtree(boundary, max_depth, max_capacity, entity_positions);

    let quadtree_shard_set_after: HashSet<u32> = quadtree
        .collect_shards()
        .iter()
        .filter_map(|s| s.shard_id)
        .collect();

    let quadtree_size_after = quadtree_shard_set_after.len();

    if quadtree_size_before != quadtree_size_after {
        let removed_shard_ids: Vec<u32> = shard_uuid_by_id
            .keys()
            .copied()
            .filter(|id| !quadtree_shard_set_after.contains(id))
            .collect();

        for id in removed_shard_ids {
            shard_uuid_by_id.remove(&id);
        }
    }
}

async fn synchronize_non_ghost_entities(
    broker_client: Option<&QuicClient>,
    quadtree: &Quadtree,
    entity_positions: &HashMap<Uuid, Vec2>,
    old_entity_positions: &HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &HashMap<u32, Uuid>,
    ghost_entity_ids: &HashMap<Uuid, HashSet<Uuid>>,
    nearby_margin: f64,
) -> anyhow::Result<()> {
    let previous_shard_ids = entity_shard_ids.clone();

    entity_shard_ids.clear();
    for (entity_id, position) in entity_positions.iter() {
        if let Some(shard_id) = quadtree.shard_for(*position) {
            entity_shard_ids.insert(*entity_id, shard_id);
        }
    }

    let no_ghost_entity_positions = entity_positions
        .iter()
        .filter(|(entity_id, _)| !ghost_entity_ids.contains_key(*entity_id))
        .map(|(entity_id, position)| (*entity_id, *position))
        .collect::<HashMap<Uuid, Vec2>>();

    for (entity_id, position) in no_ghost_entity_positions.iter() {
        let Some(old_position) = old_entity_positions.get(entity_id) else {
            continue;
        };

        if (position.x - old_position.x).abs() <= f64::EPSILON
            && (position.y - old_position.y).abs() <= f64::EPSILON
        {
            continue;
        }

        if let Some(old_nearby_shards) = ghost_entity_ids.get(entity_id) {
            let current_nearby_shards: HashSet<Uuid> = quadtree
                .shards_near(*position, nearby_margin)
                .iter()
                .filter_map(|id| shard_uuid_by_id.get(id))
                .copied()
                .collect();

            let added_nearby_shards: Vec<Uuid> = current_nearby_shards
                .difference(&old_nearby_shards)
                .copied()
                .collect();

            let owning_shard_id = entity_shard_ids.get(entity_id).unwrap().clone();
            let owning_shard_uuid = shard_uuid_by_id.get(&owning_shard_id).unwrap().clone();
            for shard_id in added_nearby_shards {
                let crossing_alert_topic = Topic::CrossingAlert(owning_shard_uuid);
                let crossing_alert_payload = CrossingAlertPayload{
                    source_shard_uuid: owning_shard_uuid,
                    target_shard_uuid: shard_id,
                    entity_uuid: *entity_id
                };
                if let Some(client) = broker_client {
                    let _ =
                        client.publish(
                            crossing_alert_topic,
                            &serialize_crossing_alert_payload(&crossing_alert_payload))
                            .await;
                }
            }
        }

        let Some(previous_shard_id) = previous_shard_ids.get(entity_id) else {
            continue;
        };

        let Some(shard_id) = entity_shard_ids.get(entity_id) else {
            continue;
        };

        if previous_shard_id == shard_id || ghost_entity_ids.contains_key(entity_id) {
            continue;
        }

        if !shard_uuid_by_id.contains_key(shard_id) {
            entity_shard_ids.insert(*entity_id, *previous_shard_id);
            continue;
        }

        if let Some(client) = broker_client {
            if let Some(previous_shard_uuid) = shard_uuid_by_id.get(previous_shard_id) {
                client
                    .unsubscribe(*previous_shard_uuid, Topic::Input(*entity_id))
                    .await?;
                client
                    .unsubscribe(*previous_shard_uuid, Topic::ForcedPositionUpdate(*entity_id))
                    .await?;
            }

            if let Some(shard_uuid) = shard_uuid_by_id.get(shard_id) {
                println!(
                    "Entity {} moved from shard {} to shard {}",
                    entity_id, previous_shard_id, shard_id
                );
                subscribe_entity_input(client, *shard_uuid, *entity_id).await?;
                subscribe_entity_position_updates(client, *shard_uuid, *entity_id).await?;

                let payload = PositionPayload {
                    entity_id: *entity_id,
                    position: *position,
                };

                let _ = client
                    .publish(
                        Topic::ForcedPositionUpdate(payload.entity_id),
                        &serialize_forced_position_update_payload(&payload),
                    )
                    .await;

                client
                    .subscribe(*entity_id, Topic::ShardSnapshot(*shard_uuid))
                    .await?;
            }
        }
    }

    Ok(())
}

async fn process_ghost_entities(
    broker_client: Option<&QuicClient>,
    quadtree: &Quadtree,
    shard_uuid_by_id: &HashMap<u32, Uuid>,
    ghost_entity_ids: &mut HashMap<Uuid, HashSet<Uuid>>,
    ghost_entity_owners: &mut HashMap<Uuid, Uuid>,
    ghost_positions: &HashMap<u32, Vec2>,
    entity_uuid_by_id: &HashMap<u32, Uuid>,
    snapshot: &common::topics::ShardSnapshotPayload,
    nearby_margin: f64,
) -> anyhow::Result<()> {
    let current_shard_uuid = snapshot.shard_id;
    let shard_id_for_current_shard = shard_uuid_by_id
        .iter()
        .find_map(|(id, uuid)| if *uuid == current_shard_uuid { Some(*id) } else { None });

    for (entity_id, pos) in ghost_positions.iter() {
        let Some(entity_uuid) = entity_uuid_by_id.get(entity_id) else {
            continue;
        };

        if ghost_entity_ids
            .get(entity_uuid)
            .map(|shard_set| shard_set.contains(&current_shard_uuid))
            .unwrap_or(false)
        {
            continue;
        }

        let Some(shard_id) = shard_id_for_current_shard else {
            continue;
        };

        if ghost_entity_owners
            .get(entity_uuid)
            .copied()
            == Some(current_shard_uuid)
        {
            continue;
        }

        let previous_nearby_shards = ghost_entity_ids
            .get(entity_uuid)
            .cloned()
            .unwrap_or_default();

        let current_nearby_shards: HashSet<Uuid> = quadtree
            .shards_near(*pos, nearby_margin)
            .iter()
            .filter_map(|id| shard_uuid_by_id.get(id))
            .copied()
            .collect();

        let removed_nearby_shards: Vec<Uuid> = previous_nearby_shards
            .difference(&current_nearby_shards)
            .copied()
            .collect();

        let transfer_target = if quadtree.position_in_inner_margin_for_shard(*pos, shard_id, nearby_margin) {
            ghost_entity_owners.get(entity_uuid).copied()
        } else {
            None
        };

        if let Some(client) = broker_client {
            for removed_shard in removed_nearby_shards {
                if Some(removed_shard) == transfer_target {
                    continue;
                }

                let payload = HandoffCompletePayload {
                    result: HandoffResult::Canceled,
                    entity_id: entity_id_from_uuid(*entity_uuid),
                    source_shard_id: removed_shard,
                    target_shard_id: current_shard_uuid,
                };

                let _ = client
                    .publish(
                        Topic::HandoffComplete(removed_shard),
                        &serialize_handoff_complete_payload(&payload),
                    )
                    .await;
            }
        }

        if !quadtree.position_in_inner_margin_for_shard(*pos, shard_id, nearby_margin) {
            continue;
        }

        let current_authority = ghost_entity_owners.get(entity_uuid).copied();

        if current_authority == Some(current_shard_uuid) {
            continue;
        }

        tracing::info!(
            "Ghost Entity {} is inside the inner margin of shard {}, transfering ownershinp from shard {:?} to shard {}",
            entity_uuid,
            shard_id,
            current_authority,
            current_shard_uuid
        );

        if let Some(client) = broker_client {
            if let Some(previous_shard_uuid) = current_authority {
                let _ = client
                    .unsubscribe(previous_shard_uuid, Topic::Input(*entity_uuid))
                    .await;
                let _ = client
                    .unsubscribe(previous_shard_uuid, Topic::ForcedPositionUpdate(*entity_uuid))
                    .await;

                let payload = HandoffCompletePayload {
                    result: HandoffResult::Transfer,
                    entity_id: entity_id_from_uuid(*entity_uuid),
                    source_shard_id: previous_shard_uuid,
                    target_shard_id: current_shard_uuid,
                };

                let _ = client
                    .publish(
                        Topic::HandoffComplete(previous_shard_uuid),
                        &serialize_handoff_complete_payload(&payload),
                    )
                    .await;
            }

            let _ = subscribe_entity_input(client, current_shard_uuid, *entity_uuid).await;
            let _ = subscribe_entity_position_updates(client, current_shard_uuid, *entity_uuid).await;

            let payload = PositionPayload {
                entity_id: *entity_uuid,
                position: *pos,
            };
            
            let _ = client
                .publish(
                    Topic::ForcedPositionUpdate(payload.entity_id),
                    &serialize_forced_position_update_payload(&payload),
                )
                .await;

            let _ = client
                .subscribe(*entity_uuid, Topic::ShardSnapshot(current_shard_uuid))
                .await;

            ghost_entity_ids
                .entry(*entity_uuid)
                .or_default()
                .insert(current_shard_uuid);
            ghost_entity_owners.insert(*entity_uuid, current_shard_uuid);
        }
    }

    Ok(())
}

async fn handle_broker_message(
    broker_client: Option<&QuicClient>,
    message: BrokerMessage,
    quadtree: &mut Quadtree,
    entity_positions: &mut HashMap<Uuid, Vec2>,
    entity_shard_ids: &mut HashMap<Uuid, u32>,
    shard_uuid_by_id: &mut HashMap<u32, Uuid>,
    ghost_entity_ids: &mut HashMap<Uuid, HashSet<Uuid>>,
    ghost_entity_owners: &mut HashMap<Uuid, Uuid>,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    nearby_margin: f64,
) -> anyhow::Result<()> {
    match message {
        BrokerMessage::Broadcast { topic, payload } => match Topic::from_bytes(topic) {
            Topic::StartingPosition => {
                let payload = deserialize_starting_position_payload(&payload)
                    .ok_or_else(|| anyhow::anyhow!("Failed to decode starting position payload"))?;
                tracing::info!("Quadtree received StartingPosition for {}", payload.entity_id);
                handle_starting_position_payload(
                    broker_client,
                    payload,
                    quadtree,
                    entity_positions,
                    entity_shard_ids,
                    shard_uuid_by_id,
                    nearby_margin,
                )
                .await?;
            }
            Topic::Disconnect(entity_id) => {
                tracing::info!("Quadtree received Disconnect for entity {}", entity_id);
                entity_positions.remove(&entity_id);
                if let Some(shard_id) = entity_shard_ids.remove(&entity_id) {
                    if let Some(shard_uuid) = shard_uuid_by_id.get(&shard_id) {
                        if let Some(client) = broker_client {
                            client.unsubscribe(*shard_uuid, Topic::Input(entity_id)).await?;
                            client.unsubscribe(*shard_uuid, Topic::ForcedPositionUpdate(entity_id)).await?;
                        }
                    }
                }
            }
            Topic::ShardCreated => {
                let payload: ShardCreatedPayload = deserialize_shard_created_payload(&payload)
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
                //tracing::info!("Quadtree received ShardSnapshot");
                handle_shard_snapshot_payload(
                    payload,
                    quadtree,
                    boundary,
                    max_depth,
                    max_capacity,
                    entity_positions,
                    entity_shard_ids,
                    shard_uuid_by_id,
                    ghost_entity_ids,
                    ghost_entity_owners,
                    broker_client,
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
    ghost_entity_ids: &mut HashMap<Uuid, HashSet<Uuid>>,
    ghost_entity_owners: &mut HashMap<Uuid, Uuid>,
    boundary: Boundary,
    max_depth: u8,
    max_capacity: usize,
    //shards: &mut HashSet<u32>,
    nearby_margin: f64,
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
                        ghost_entity_ids,
                        ghost_entity_owners,
                        boundary,
                        max_depth,
                        max_capacity,
                        nearby_margin,
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
        if let Err(e) = client.subscribe(quadtree_id, Topic::StartingPosition).await {
            tracing::warn!("Failed to subscribe to StartingPosition: {e}");
        } else {
            tracing::info!("Quadtree subscribed to StartingPosition");
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
    let mut ghost_entity_ids: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
    let mut ghost_entity_owners: HashMap<Uuid, Uuid> = HashMap::new();

    //simulate a publish-subscribe system where entities are added to the quadtree and we query for nearby shards
    loop {
        if let Some(client) = broker_client.as_mut() {
            process_broker_events(
                client,
                &mut quadtree,
                &mut entity_positions,
                &mut entity_shard_ids,
                &mut shard_uuid_by_id,
                &mut ghost_entity_ids,
                &mut ghost_entity_owners,
                boundary,
                config.max_depth,
                config.max_capacity,
               // &mut shards,
                config.nearby_margin,
            )
            .await?;
        }

        // Query Shard Data that will be sent to the orchestrator to update server layout
        let shard_data = quadtree.collect_shards();
        // Check if the shard layout has changed and if so, update the `shards` set and print the new layout
        let new_shard_ids: std::collections::HashSet<u32> = shard_data.iter().filter_map(|s| s.shard_id).collect();

        //println!("Previous shard IDs: {:?}", shards);
        //println!("Current shard IDs: {:?}", new_shard_ids);
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
        /*
        println!("Current Shard Layout:");
        for shard in &shard_data {
            println!("Shard ID: {:?}, Boundary: center=({:.2}, {:.2}), half_size={:.2}", shard.shard_id, shard.boundary.x, shard.boundary.y, shard.boundary.half_size);
        }*/

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

        // remove the shard in which `pos` is located so the shard with authority doesn't get ghost updates (to avoid flickering )
        if let Some(shard_id) = self.shard_for(pos) {
            shards.retain(|id| *id != shard_id); // we will see later for authority margin management
        }
        println!("shards_near: pos=({:.2}, {:.2}), margin={:.2} => nearby shard IDs = {:?}", pos.x, pos.y, margin, shards);
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

    fn shard_center(&self, shard_id: u32) -> Option<Vec2> {
        if let Some(id) = self.shard_id {
            if id == shard_id {
                return Some(Vec2 {
                    x: self.boundary.x,
                    y: self.boundary.y,
                });
            }
        }

        if let Some(children) = &self.children {
            for child in children {
                if let Some(center) = child.shard_center(shard_id) {
                    return Some(center);
                }
            }
        }

        None
    }

    fn position_in_inner_margin_for_shard(&self, ghost_pos: Vec2, shard_id: u32, margin: f64) -> bool {
            if let Some(center) = self.shard_center(shard_id) {
                println!("center for shard {} is ({:.2}, {:.2})", shard_id, center.x, center.y);
                let dx = (ghost_pos.x - center.x).abs();
                let dy = (ghost_pos.y - center.y).abs();
                let half_size = self.boundary.half_size;
                return dx < half_size - margin && dy < half_size - margin;
            }
        false
    }

}