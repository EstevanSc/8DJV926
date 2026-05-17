#!/bin/bash

# Mock Dedicated Server - Simulates heartbeat messages
# Usage: ./mock_server.sh [SERVER_ID] [PORT]
# Example: ./mock_server.sh server-mock-1 7777

SERVER_ID="${1:-server-mock-1}"
PORT="${2:-7777}"

HEARTBEAT_LISTENER_IP="127.0.0.1"
HEARTBEAT_LISTENER_PORT="7000"
HEARTBEAT_INTERVAL=5

echo "[Mock Server] Starting with ID=$SERVER_ID, PORT=$PORT"
echo "[Mock Server] Sending heartbeats to $HEARTBEAT_LISTENER_IP:$HEARTBEAT_LISTENER_PORT every ${HEARTBEAT_INTERVAL}s"

while true; do
    HEARTBEAT="{\"id\":\"$SERVER_ID\",\"ip\":\"127.0.0.1\",\"port\":$PORT,\"zone\":\"zone_a\",\"player_count\":0,\"max_players\":100}"
    
    printf '%s' "$HEARTBEAT" > /dev/udp/$HEARTBEAT_LISTENER_IP/$HEARTBEAT_LISTENER_PORT
    echo "[Mock Server] Sent heartbeat: $HEARTBEAT"
    
    sleep "$HEARTBEAT_INTERVAL"
done
