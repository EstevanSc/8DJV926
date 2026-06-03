use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum BrokerMessage {
    Subscribe { client_id: Uuid, topic: [u8; 32] },         // 0x01
    Unsubscribe { client_id: Uuid, topic: [u8; 32] },       // 0x02
    Publish { topic: [u8; 32], payload: Vec<u8> },          // 0x03
    Broadcast { topic: [u8; 32], payload: Vec<u8> },        // 0x04
    Connect { client_id: Uuid , sending_system: SendingSystem },                            // 0x05
}   

#[derive(Debug, Clone)]
pub enum SendingSystem {
    Quadtree,
    Server,
    Orchestrator,
    Client,
    Gatekeeper,
}

impl BrokerMessage {
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.is_empty() {
            return None;
        }

        let tag = data[0];
        let body = &data[1..];

        match tag {
            0x01 => { // Subscribe: client_id (16B) + topic (32B)
                if body.len() < 48 {
                    return None;
                }

                let client_id = Uuid::from_slice(&body[0..16]).ok()?;
                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[16..48]);
                Some(BrokerMessage::Subscribe { client_id, topic })
            }
            0x02 => { // Unsubscribe: client_id (16B) + topic (32B)
                if body.len() < 48 {
                    return None;
                }

                let client_id = Uuid::from_slice(&body[0..16]).ok()?;
                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[16..48]);
                Some(BrokerMessage::Unsubscribe { client_id, topic })
            }
            0x03 => { // Publish: topic (32B) + payload_len (2B) + payload
                if body.len() < 34 {
                    return None;
                }

                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[0..32]);
                let payload_len = u16::from_le_bytes(body[32..34].try_into().ok()?) as usize;

                if body.len() < 34 + payload_len {
                    return None;
                }

                let payload = body[34..34 + payload_len].to_vec();
                Some(BrokerMessage::Publish { topic, payload })
            }
            0x04 => { // Broadcast: topic (32B) + payload_len (2B) + payload // Note: This message type is only sent by the broker, the other parts use this deserialization.
                if body.len() < 34 {
                    return None;
                }

                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[0..32]);
                let payload_len = u16::from_le_bytes(body[32..34].try_into().ok()?) as usize;

                if body.len() < 34 + payload_len {
                    return None;
                }

                let payload = body[34..34 + payload_len].to_vec();
                Some(BrokerMessage::Broadcast { topic, payload })
            }
                        0x05 => { // Connect: client_id (16B)
                if body.len() < 16 {
                    return None;
                }
                let client_id = Uuid::from_slice(&body[0..16]).ok()?;
                let sending_system = match body[16] {
                    0x01 => SendingSystem::Quadtree,
                    0x02 => SendingSystem::Server,
                    0x03 => SendingSystem::Orchestrator,
                    0x04 => SendingSystem::Client,
                    0x05 => SendingSystem::Gatekeeper,
                    _ => return None,
                };
                Some(BrokerMessage::Connect { client_id, sending_system })
            }
            _ => None,
        }
    }

    pub fn serialize_publish(topic: [u8; 32], payload: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1 + 32 + 2 + payload.len());
        buffer.push(0x03);
        buffer.extend_from_slice(&topic);
        buffer.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buffer.extend_from_slice(payload);
        buffer
    }

    pub fn serialize_subscribe(client_id: Uuid, topic: [u8; 32]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1 + 16 + 32);
        buffer.push(0x01);
        buffer.extend_from_slice(client_id.as_bytes());
        buffer.extend_from_slice(&topic);
        buffer
    }

    pub fn serialize_unsubscribe(client_id: Uuid, topic: [u8; 32]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1 + 16 + 32);
        buffer.push(0x02);
        buffer.extend_from_slice(client_id.as_bytes());
        buffer.extend_from_slice(&topic);
        buffer
    }

    pub fn serialize_connect(client_id: Uuid, sending_system: SendingSystem) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1 + 16 + 1);
        buffer.push(0x05);
        buffer.extend_from_slice(client_id.as_bytes());
        let system_byte = match sending_system {
            SendingSystem::Quadtree => 0x01,
            SendingSystem::Server => 0x02,
            SendingSystem::Orchestrator => 0x03,
            SendingSystem::Client => 0x04,
            SendingSystem::Gatekeeper => 0x05,
        };
        buffer.push(system_byte);
        buffer
    }

    pub fn serialize_broadcast(topic: [u8; 32], payload: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(1 + 32 + 2 + payload.len());
        buffer.push(0x04);
        buffer.extend_from_slice(&topic);
        buffer.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        buffer.extend_from_slice(payload);
        buffer
    }
}