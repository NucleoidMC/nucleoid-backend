use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ServerStatus {
    pub game_version: String,
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
pub struct ChatAttachment {
    pub name: String,
    pub url: String,
}
