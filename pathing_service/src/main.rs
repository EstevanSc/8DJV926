mod quic_client;
mod config;
mod pathfinding;

use quic_client::QuicClient;
use crate::config::Config;
use pathfinding::{Graph, Node, run_a_star, build_portals_from_nodes, run_funnel, has_line_of_sight};

use std::time::{Duration};

use common::topics::{
      Topic, PathRequestPayload, PathResponsePayload, deserialize_path_request_payload, serialize_path_response_payload
};

use common::BrokerMessage;
use common::map_data::{BitMap, MAP_HEIGHT, MAP_WIDTH, TILE_SIZE};
use game_sockets::GameNetworkEvent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let config = Config::from_env();

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

    run_main_loop(
        broker_client,
    )
    .await
}


async fn run_main_loop(
    broker_client: Option<QuicClient>,
) -> anyhow::Result<()> {

    let mut broker_client = broker_client;

    if let Some(client) = broker_client.as_ref() {
        let client_id = client.connection_id();
        client.announce_connect(client_id).await?;
        client.subscribe(client_id, Topic::PathRequest).await?;
        tracing::info!(
            "Subscribed pathing service to broker topics {:?} with client_id={}",
            Topic::PathRequest,
            client_id
        );
    }

    let mut map = BitMap::new();
    map.generate_map();

    let mut graph = Graph::new();

    // 1. First Pass: Register all walkable nodes
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if !map.is_wall(x, y) {
                let node_id = y * MAP_WIDTH + x;
                graph.nodes.insert(node_id, Node { id: node_id, x, y });
                graph.adjacency_list.insert(node_id, Vec::new());
            }
        }
    }

    // 2. Second Pass: Find neighbors for each walkable node (4-way movement)
    let directions: [(i32, i32); 4] = [
        (0, -1), // Up
        (0, 1),  // Down
        (-1, 0), // Left
        (1, 0),  // Right
    ];

    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            if map.is_wall(x, y) {
                continue; // Skip wall nodes entirely
            }

            let current_id = y * MAP_WIDTH + x;

            for (dx, dy) in directions.iter() {
                let target_x = x as i32 + dx;
                let target_y = y as i32 + dy;

                // Ensure neighbor is within map boundaries
                if target_x >= 0 && target_x < MAP_WIDTH as i32 && target_y >= 0 && target_y < MAP_HEIGHT as i32 {
                    let tx = target_x as usize;
                    let ty = target_y as usize;

                    // If the neighbor is walkable, add an edge
                    if !map.is_wall(tx, ty) {
                        let neighbor_id = ty * MAP_WIDTH + tx;
                        if let Some(neighbors) = graph.adjacency_list.get_mut(&current_id) {
                            neighbors.push(neighbor_id);
                        }
                    }
                }
            }
        }
    }

    let mut tick = tokio::time::interval(Duration::from_millis(5));

    loop {
        tick.tick().await;
        
        if let Some(client) = broker_client.as_mut() {
            poll_quic_events(client, "broker", &graph).await?;
        }
    }
}

async fn poll_quic_events(
    broker: &mut QuicClient,
    label: &str,
    graph: &Graph,
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

                handle_quic_message(&data, broker, graph).await;
            }
            _ => {}
        }
    }

    Ok(())
}

async fn handle_quic_message(
    data: &[u8],
    broker: &QuicClient,
    graph: &Graph,
) {
    let Some(message) = BrokerMessage::deserialize(data) else {
        return;
    };

    match message {
        BrokerMessage::Broadcast { topic, payload }
        | BrokerMessage::Publish { topic, payload } => {
            match Topic::from_bytes(topic) {
                Topic::PathRequest => {
                    if let Some(path_request) = deserialize_path_request_payload(&payload) {
                        handle_path_request(&path_request, broker, graph).await;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

async fn handle_path_request(
    request: &PathRequestPayload,
    broker: &QuicClient,
    graph: &Graph,
) {
    tracing::info!("Received path request for entity {} from {:?} to {:?}", request.entity_id, request.start, request.end);

    // Calculate the map's world-space offset (assuming Bevy 0,0 is the center of the map)
    let map_world_width = MAP_WIDTH as f32 * TILE_SIZE;
    let map_world_height = MAP_HEIGHT as f32 * TILE_SIZE;
    let offset_x = map_world_width / 2.0;
    let offset_y = map_world_height / 2.0;

    // Shift coordinates to be positive relative to the grid's top-left (or bottom-left)
    let shifted_start_x = request.start[0] + offset_x;
    let shifted_start_y = request.start[1] + offset_y;
    let shifted_end_x = request.end[0] + offset_x;
    let shifted_end_y = request.end[1] + offset_y;

    // Boundary check to prevent crashing or underflowing if clicking outside the map
    if shifted_start_x < 0.0 || shifted_start_y < 0.0 || shifted_end_x < 0.0 || shifted_end_y < 0.0 ||
       shifted_start_x >= map_world_width || shifted_start_y >= map_world_height || 
       shifted_end_x >= map_world_width || shifted_end_y >= map_world_height 
    {
        tracing::warn!("Path request out of bounds");
        return; // Or send a fallback response
    }

    let start_x = (shifted_start_x / TILE_SIZE) as usize;
    let start_y = (shifted_start_y / TILE_SIZE) as usize;
    let end_x = (shifted_end_x / TILE_SIZE) as usize;
    let end_y = (shifted_end_y / TILE_SIZE) as usize;

    let start_id = start_y * MAP_WIDTH + start_x;
    let end_id = end_y * MAP_WIDTH + end_x;

    // THE LINE OF SIGHT SHORTCUT
    let smooth_path = if has_line_of_sight(graph, MAP_WIDTH, start_x, start_y, end_x, end_y) {
        tracing::info!("Clear line of sight! Bypassing A*.");
        vec![request.start, request.end]
    } 
    // If blocked, fall back to normal A* routing
    else if let Some(node_path) = run_a_star(graph, start_id, end_id) {
        let portals = build_portals_from_nodes(&node_path, graph, TILE_SIZE, offset_x, offset_y);
        run_funnel(request.start, request.end, &portals)
    } else {
        vec![request.start, request.end]
    };

    let topic = Topic::PathResponse(request.entity_id);
    let response = PathResponsePayload { path: smooth_path };

    tracing::info!("Publishing path response for entity {} with waypoints: {:?}", request.entity_id, response.path);

    let response_bytes = serialize_path_response_payload(&response);
    if let Err(e) = broker.publish(topic, &response_bytes).await {
        tracing::error!("Failed to publish path response for entity {}: {}", request.entity_id, e);
    }
}
