use std::fs::File;
use std::future::Future;
use std::path::Path;

use serde::{Deserialize, Serialize};
use xtra::prelude::*;
use xtra::spawn::Spawner;

pub use controller::*;
pub use persistent::*;

// mod web;
mod integrations;
mod discord;
mod model;
mod controller;
mod persistent;

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub web_server_port: u16,
    pub integrations_port: u16,
    pub discord_token: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            web_server_port: 25010,
            integrations_port: 25020,
            discord_token: String::new(),
        }
    }
}

fn load_config() -> Config {
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

pub struct TokioGlobal;

impl Spawner for TokioGlobal {
    fn spawn<F: Future<Output = ()> + Send + 'static>(&mut self, fut: F) {
        tokio::spawn(fut);
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let config = load_config();
    let controller = Controller::new(config.clone())
        .create(None)
        .spawn(&mut TokioGlobal);

    let _ = futures::future::join(
        tokio::spawn(integrations::run(controller.clone(), config.clone())),
        tokio::spawn(discord::run(controller.clone(), config.clone())),
    ).await;
}
