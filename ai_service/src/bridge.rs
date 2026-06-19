use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use common::Boundary;
use tokio::runtime::Runtime;
use uuid::Uuid;

use common::topics::{
    deserialize_position_payload, deserialize_quadtree_boundaries_update_payload,
    serialize_input_payload, InputPayload, Topic,
    PathRequestPayload, serialize_path_request_payload,
    deserialize_path_response_payload,
    UseAbilityPayload, serialize_use_ability_payload,
};

use crate::client::{AiClient, ClientPool, InboundMessage, MasterClient};
use crate::components::{AiEntity, AiIntent, AiPosition, Perception, AiPath};
use crate::config::Config;

/// Bevy resource wrapping the tokio runtime.
#[derive(Resource)]
pub struct TokioRuntime(pub Arc<Runtime>);

/// Bevy resource holding all active AI clients.
#[derive(Resource, Default)]
pub struct AiClients(pub Arc<Mutex<ClientPool>>);

/// Bevy resource holding inbound message receivers, keyed by AI UUID.
#[derive(Resource, Default)]
pub struct InboundReceivers(
    pub HashMap<Uuid, tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>,
);

/// Bevy resource tracking live shard boundaries from the Quadtree.
#[derive(Resource, Default)]
pub struct QuadtreeBoundaries(pub Vec<Boundary>);

/// Bevy resource holding the master client dedicated to quadtree topologies.
#[derive(Resource)]
pub struct MasterClientResource {
    pub client: Arc<MasterClient>,
}

/// Bevy resource holding the receiver for the master client.
#[derive(Resource)]
pub struct MasterInbound(pub std::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<InboundMessage>>);

/// Bevy plugin that connects new AI entities to the broker and routes messages.
pub struct BridgePlugin;

impl Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiClients>()
            .init_resource::<InboundReceivers>()
            .init_resource::<QuadtreeBoundaries>()
            .add_systems(Startup, setup_master_client)
            .add_systems(Update, (
                poll_master_client,
                poll_clients,
                connect_new_entities,
                drain_inbound,
                flush_intents
            ).chain());
    }
}

/// Initializes the master client dedicated to quadtree monitoring.
fn setup_master_client(mut commands: Commands, config: Res<Config>, runtime: Res<TokioRuntime>) {
    let id = Uuid::new_v4();
    let host = config.broker_host.clone();
    let port = config.broker_port;

    let (client, rx) = runtime
        .0
        .block_on(MasterClient::connect(id, &host, port))
        .unwrap_or_else(|e| panic!("Failed to connect MasterClient: {e}"));

    commands.insert_resource(MasterClientResource { client: Arc::new(client) });
    commands.insert_resource(MasterInbound(std::sync::Mutex::new(rx)));

    tracing::info!("MasterClient {id} connected to broker at {host}:{port} for Quadtree updates");
}

/// Polls the master client and updates the QuadtreeBoundaries resource.
fn poll_master_client(
    master: Option<Res<MasterClientResource>>,
    inbound: Option<Res<MasterInbound>>,
    mut boundaries: ResMut<QuadtreeBoundaries>,
) {
    let (Some(master), Some(inbound)) = (master, inbound) else { return };
    
    master.client.poll();

    let mut rx = inbound.0.lock().unwrap();
    while let Ok(msg) = rx.try_recv() {
        if let Topic::QuadtreeBoundariesUpdate = Topic::from_bytes(msg.topic) {
            if let Some(payload) = deserialize_quadtree_boundaries_update_payload(&msg.payload) {
                boundaries.0 = payload.boundaries;
            }
        }
    }
}

/// Polls every active AI client for inbound broker messages.
fn poll_clients(clients: Res<AiClients>) {
    clients.0.lock().unwrap().poll_all();
}

/// Detects newly spawned AiEntity components and opens a QUIC connection for each.
fn connect_new_entities(
    query: Query<(&AiEntity, &AiPosition), Added<AiEntity>>,
    config: Res<Config>,
    runtime: Res<TokioRuntime>,
    clients: Res<AiClients>,
    mut receivers: ResMut<InboundReceivers>,
) {
    for (ai, pos) in &query {
        let id = ai.id;
        let host = config.broker_host.clone();
        let port = config.broker_port;
        let starting_pos = [pos.x, pos.y];
        let pool = Arc::clone(&clients.0);

        let (client, rx) = runtime
            .0
            .block_on(AiClient::connect(id, &host, port, 
                [starting_pos[0] as f64, starting_pos[1] as f64]))
            .unwrap_or_else(|e| panic!("Failed to connect AI {id}: {e}"));

        pool.lock().unwrap().clients.insert(id, client);
        receivers.0.insert(id, rx);

        tracing::info!("AI {id} connected to broker at {host}:{port}");
    }
}

/// Drains inbound broker messages and updates AiPosition / Perception components.
fn drain_inbound(
    mut receivers: ResMut<InboundReceivers>,
    mut query: Query<(&AiEntity, &mut AiPosition, &mut Perception, &mut AiPath)>,
) {
    for (ai, mut pos, mut perception, mut path) in &mut query {
        let Some(rx) = receivers.0.get_mut(&ai.id) else { continue };

        while let Ok(msg) = rx.try_recv() {
            tracing::debug!("AI {} received topic {:?}", ai.id, Topic::from_bytes(msg.topic));
            if let Topic::EntityPositionUpdate(sender) = Topic::from_bytes(msg.topic) {
                if let Some(update) = deserialize_position_payload(&msg.payload) {
                    if sender == ai.id {
                        pos.x = update.position[0] as f32;
                        pos.y = update.position[1] as f32;
                        // if the position is near enough of the current path target, pop it from the path
                        if let Some(target) = path.waypoints.first() {
                            let dist2 = (target[0] - pos.x).powi(2) + (target[1] - pos.y).powi(2);
                            if dist2 < 10.0f32.powi(2) {
                                path.waypoints.remove(0);
                            }
                        }
                    } else {
                        upsert_nearby(&mut perception, sender, [update.position[0] as f32, update.position[1] as f32]);
                    }
                }
            }
            if let Topic::PathResponse(id) = Topic::from_bytes(msg.topic) {
                if id == ai.id {
                    if let Some(update) = deserialize_path_response_payload(&msg.payload) {
                        // Handle path response 
                        tracing::debug!("AI {} received path response: {:?}", ai.id, update.path);
                        path.waypoints = update.path;
                    }
                }
            }
        }
    }
}

/// Inserts or updates a nearby entity's position in the Perception list.
fn upsert_nearby(perception: &mut Perception, id: Uuid, pos: [f32; 2]) {
    tracing::info!("Updating perception of nearby entity {} at position {:?}", id, pos);
    match perception.nearby.iter_mut().find(|(eid, _)| *eid == id) {
        Some(entry) => entry.1 = pos,
        None => perception.nearby.push((id, pos)),
    }
}

/// Reads AiIntent components and publishes the corresponding broker messages.
fn flush_intents(
    mut query: Query<(&AiEntity, &mut AiPosition, &mut AiIntent, &mut AiPath)>,
    clients: Res<AiClients>,
) {
    let pool = clients.0.lock().unwrap();

    for (ai, position, mut intent, path) in &mut query {
        let Some(client) = pool.clients.get(&ai.id) else { continue };

        match *intent {
            AiIntent::MoveTo(target) => {
                // Send a payload to the pathfinding service to get a path, then publish Input messages to move along that path.
                client.publish(Topic::PathRequest.to_bytes(), &serialize_path_request_payload(&PathRequestPayload {
                    entity_id: ai.id,
                    start: [position.x as f32, position.y as f32],
                    end: [target[0] as f32, target[1] as f32],
                }));

                let current_target = path.waypoints.first().cloned().unwrap_or([target[0] as f32, target[1] as f32]);
                let dir = [current_target[0] - position.x, current_target[1] - position.y];
                let mag = (dir[0].powi(2) + dir[1].powi(2)).sqrt();
                let dir = if mag > 0.0 { [dir[0] / mag, dir[1] / mag] } else { [0.0, 0.0] };
                tracing::debug!("AI {} moving toward {:?}, dir {:?}", ai.id, current_target, dir);
                let payload = serialize_input_payload(&InputPayload { dxdy: [dir[0] as f64, dir[1] as f64] });
                client.publish(Topic::Input(ai.id).to_bytes(), &payload);
                *intent = AiIntent::Idle;
            }
            AiIntent::CastAbility(ability, direction) => {
                let payload = serialize_use_ability_payload(&UseAbilityPayload {
                    entity_id: ai.id,
                    ability,
                    direction,
                });
                client.publish(Topic::CastAbility(ai.id).to_bytes(), &payload);
                *intent = AiIntent::Idle;
            }
            AiIntent::Idle => {}
        }
    }
}