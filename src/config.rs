use std::fs::File;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    pub web_server: Option<WebServerConfig>,
    pub integrations: Option<IntegrationsConfig>,
    pub discord: Option<DiscordConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            web_server: Some(WebServerConfig { port: 25010 }),
            integrations: Some(IntegrationsConfig { port: 25020 }),
            discord: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiscordConfig {
    pub token: String,
    pub ping_interval_minutes: u16,
    #[serde(default = true)]
    pub relay_channel_topic: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WebServerConfig {
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct IntegrationsConfig {
    pub port: u16,
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
