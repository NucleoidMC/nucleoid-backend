use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServerStatus {
    pub game_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_ip: Option<String>,
    pub games: Vec<Game>,
    pub players: Vec<Player>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Player {
    pub id: String,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Game {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    pub player_count: u16,
}

#[derive(Serialize, Debug)]
pub struct ChatMessage {
    pub sender: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_color: Option<u32>,
    pub attachments: Vec<ChatAttachment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replying_to: Option<Box<ChatMessage>>,
}

#[derive(Serialize, Debug)]
pub struct ChatAttachment {
    pub name: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServerPerformance {
    pub average_tick_ms: f32,
    pub tps: u8,
    pub dimensions: u16,
    pub entities: u32,
    pub chunks: u32,
    pub used_memory: u64,
    pub total_memory: u64,
}
