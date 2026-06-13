use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use tokio::runtime::Runtime;
use uuid::Uuid;

use common::topics::{
    deserialize_position_payload, serialize_input_payload, InputPayload, Topic,
};

use crate::client::{AiClient, ClientPool, InboundMessage};
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

/// Bevy plugin that connects new AI entities to the broker and routes messages.
pub struct BridgePlugin;

impl Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiClients>()
            .init_resource::<InboundReceivers>()
            .add_systems(Update, (poll_clients, connect_new_entities, drain_inbound, flush_intents).chain());
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
        let ip = config.broker_ip.clone();
        let port = config.broker_port;
        let starting_pos = [pos.x, pos.y];
        let pool = Arc::clone(&clients.0);

        let (client, rx) = runtime
            .0
            .block_on(AiClient::connect(id, &ip, port, starting_pos))
            .unwrap_or_else(|e| panic!("Failed to connect AI {id}: {e}"));

        pool.lock().unwrap().clients.insert(id, client);
        receivers.0.insert(id, rx);

        tracing::info!("AI {id} connected to broker at {ip}:{port}");
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
    mut query: Query<(&AiEntity, &mut AiIntent)>,
    clients: Res<AiClients>,
) {
    let pool = clients.0.lock().unwrap();

    for (ai, mut intent) in &mut query {
        let Some(client) = pool.clients.get(&ai.id) else { continue };

        match *intent {
            AiIntent::MoveTo(target) => {
                let dir = [target[0].signum(), target[1].signum()];
                tracing::debug!("AI {} moving toward {:?}, dir {:?}", ai.id, target, dir);
                let payload = serialize_input_payload(&InputPayload { dxdy: dir });
                client.publish(Topic::Input(ai.id).to_bytes(), &payload);
                *intent = AiIntent::Idle;
            }
            AiIntent::Idle => {}
        }
    }
}