use std::fs::File;
use std::path::Path;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    pub web_server: Option<WebServerConfig>,
    pub integrations: Option<IntegrationsConfig>,
    pub discord: Option<DiscordConfig>,
    pub database: Option<DatabaseConfig>,
    #[serde(default = "HashMap::new()")]
    pub kickbacks: HashMap<String, Kickback>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiscordConfig {
    pub token: String,
    pub ping_interval_minutes: u16,
    #[serde(default)]
    pub relay_channel_topic: bool,
    #[serde(default)]
    pub player_avatar_url: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WebServerConfig {
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IntegrationsConfig {
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Kickback {
    pub to_server: String,
    pub from_server: String,
    pub proxy_channel: String,
}

pub(super) fn load() -> Config {
    let path = Path::new("config.json");
    if path.exists() {
        let mut file = File::open(path).expect("failed to open config");
        serde_json::from_reader(&mut file).expect("failed to parse config")
    } else {
        let config = Config::default();

        let mut file = File::create(path).expect("failed to create config");
        serde_json::to_writer_pretty(&mut file, &config).expect("failed to write config");

        config
    }
}
