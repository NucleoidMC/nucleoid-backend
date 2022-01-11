use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Config {
    pub web_server: Option<WebServerConfig>,
    pub integrations: Option<IntegrationsConfig>,
    pub discord: Option<DiscordConfig>,
    pub database: Option<DatabaseConfig>,
    #[serde(default = "HashMap::new")]
    pub kickbacks: HashMap<String, Kickback>,
    pub statistics: Option<StatisticsConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiscordConfig {
    pub token: String,
    #[serde(default = "default_ping_interval_minutes")]
    pub ping_interval_minutes: u16,
    #[serde(default = "default_lfp_ping_interval_minutes")]
    pub lfp_ping_interval_minutes: u16,
    #[serde(default)]
    pub relay_channel_topic: bool,
    #[serde(default)]
    pub player_avatar_url: Option<String>,
    #[serde(default)]
    pub error_webhook: Option<ErrorWebhookConfig>,
}

fn default_ping_interval_minutes() -> u16 { 30 }

fn default_lfp_ping_interval_minutes() -> u16 { 10 }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ErrorWebhookConfig {
    pub id: u64,
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WebServerConfig {
    pub port: u16,
    pub max_query_size: u32,
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StatisticsConfig {
    pub database_url: String,
    pub database_name: String,
    pub leaderboards_dir: Option<PathBuf>,
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
