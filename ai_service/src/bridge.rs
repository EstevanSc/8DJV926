use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use common::Boundary;
use tokio::runtime::Runtime;
use uuid::Uuid;

use common::topics::{
    deserialize_position_payload, deserialize_quadtree_boundaries_update_payload,
    serialize_input_payload, InputPayload, Topic,
};

use crate::client::{AiClient, ClientPool, InboundMessage, MasterClient};
use crate::components::{AiEntity, AiIntent, AiPosition, Perception};
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
            .block_on(AiClient::connect(id, &host, port, starting_pos))
            .unwrap_or_else(|e| panic!("Failed to connect AI {id}: {e}"));

        pool.lock().unwrap().clients.insert(id, client);
        receivers.0.insert(id, rx);

        tracing::info!("AI {id} connected to broker at {host}:{port}");
    }
}

/// Drains inbound broker messages and updates AiPosition / Perception components.
fn drain_inbound(
    mut receivers: ResMut<InboundReceivers>,
    mut query: Query<(&AiEntity, &mut AiPosition, &mut Perception)>,
) {
    for (ai, mut pos, mut perception) in &mut query {
        let Some(rx) = receivers.0.get_mut(&ai.id) else { continue };

        while let Ok(msg) = rx.try_recv() {
            tracing::debug!("AI {} received topic {:?}", ai.id, Topic::from_bytes(msg.topic));
            if let Topic::EntityPositionUpdate(sender) = Topic::from_bytes(msg.topic) {
                if let Some(update) = deserialize_position_payload(&msg.payload) {
                    if sender == ai.id {
                        pos.x = update.position[0];
                        pos.y = update.position[1];
                    } else {
                        upsert_nearby(&mut perception, sender, update.position);
                    }
                }
            }
        }
    }
}

/// Inserts or updates a nearby entity's position in the Perception list.
fn upsert_nearby(perception: &mut Perception, id: Uuid, pos: [f64; 2]) {
    match perception.nearby.iter_mut().find(|(eid, _)| *eid == id) {
        Some(entry) => entry.1 = pos,
        None => perception.nearby.push((id, pos)),
    }
}

/// Reads AiIntent components and publishes the corresponding broker messages.
fn flush_intents(
    mut query: Query<(&AiEntity, &mut AiPosition, &mut AiIntent)>,
    clients: Res<AiClients>,
) {
    let pool = clients.0.lock().unwrap();

    for (ai, position, mut intent) in &mut query {
        let Some(client) = pool.clients.get(&ai.id) else { continue };

        match *intent {
            AiIntent::MoveTo(target) => {
                let dir = [target[0] - position.x, target[1] - position.y];
                let mag = (dir[0].powi(2) + dir[1].powi(2)).sqrt();
                let dir = if mag > 0.0 { [dir[0] / mag, dir[1] / mag] } else { [0.0, 0.0] };
                tracing::debug!("AI {} moving toward {:?}, dir {:?}", ai.id, target, dir);
                let payload = serialize_input_payload(&InputPayload { dxdy: dir });
                client.publish(Topic::Input(ai.id).to_bytes(), &payload);
                *intent = AiIntent::Idle;
            }
            AiIntent::Idle => {}
        }
    }
}