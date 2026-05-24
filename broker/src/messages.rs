#[derive(Debug, Clone)]
pub enum BrokerMessage {
    Subscribe { client_id: u32, topic: [u8; 32] },         // 0x01
    Unsubscribe { client_id: u32, topic: [u8; 32] },       // 0x02
    Publish { topic: [u8; 32], payload: Vec<u8> },         // 0x03
    Broadcast { payload: Vec<u8> },                        // 0x04
    ClientInput { client_id: u32, input: [u8; 16] },       // 0x05
}

impl BrokerMessage {
    pub fn deserialize(data: &[u8]) -> Option<Self> {
        if data.is_empty() { return None; }
        let tag = data[0];
        let body = &data[1..];

        match tag {
            0x01 => { // Subscribe: client_id (4B) + topic (32B)
                if body.len() < 36 { return None; }
                let client_id = u32::from_le_bytes(body[0..4].try_into().unwrap());
                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[4..36]);
                Some(BrokerMessage::Subscribe { client_id, topic })
            }
            0x02 => { // Unsubscribe: client_id (4B) + topic (32B)
                if body.len() < 36 { return None; }
                let client_id = u32::from_le_bytes(body[0..4].try_into().unwrap());
                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[4..36]);
                Some(BrokerMessage::Unsubscribe { client_id, topic })
            }
            0x03 => { // Publish: topic (32B) + payload_len (2B) + payload
                if body.len() < 34 { return None; }
                let mut topic = [0u8; 32];
                topic.copy_from_slice(&body[0..32]);
                let payload_len = u16::from_le_bytes(body[32..34].try_into().unwrap()) as usize;

                if body.len() < 34 + payload_len { return None; }
                let payload = body[34..34 + payload_len].to_vec();
                Some(BrokerMessage::Publish { topic, payload })
            }
            0x05 => { // ClientInput: client_id (4B) + input (16B)
                if body.len() < 20 { return None; }
                let client_id = u32::from_le_bytes(body[0..4].try_into().unwrap());
                let mut input = [0u8; 16];
                input.copy_from_slice(&body[4..20]);
                Some(BrokerMessage::ClientInput { client_id, input })
            }
            _ => None, // 0x04 (Broadcast) is sent outbound only; shouldn't be received
        }
    }

    /// Serializes a Broadcast message into the exact raw payload required by the client
    pub fn serialize_broadcast(payload: &[u8]) -> Vec<u8> {
        let mut buffer = Vec::with_capacity(size_of::<u8>() + size_of::<u16>() + payload.len());
        buffer.push(0x04); // Tag
        buffer.extend_from_slice(&(payload.len() as u16).to_le_bytes()); // payload_len
        buffer.extend_from_slice(payload);
        buffer
    }
}