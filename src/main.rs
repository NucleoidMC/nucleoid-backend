use std::future::Future;

use xtra::prelude::*;
use xtra::spawn::Spawner;

pub use config::*;
pub use controller::*;
pub use persistent::*;

mod web;
mod integrations;
mod discord;
mod model;
mod controller;
mod persistent;
mod config;
mod database;

pub struct TokioGlobal;

impl Spawner for TokioGlobal {
    fn spawn<F: Future<Output = ()> + Send + 'static>(&mut self, fut: F) {
        tokio::spawn(fut);
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let config = config::load();
    let controller = Controller::new(config.clone()).await
        .create(None)
        .spawn(&mut TokioGlobal);

    let mut futures = Vec::with_capacity(3);

    if let Some(integrations) = config.integrations {
        futures.push(tokio::spawn(integrations::run(controller.clone(), integrations)));
    }

    if let Some(web) = config.web_server {
        futures.push(tokio::spawn(web::run(controller.clone(), web)));
    }

    if let Some(discord) = config.discord {
        futures.push(tokio::spawn(discord::run(controller.clone(), discord)));
    }

    if let Some(database) = config.database {
        futures.push(tokio::spawn(database::run(controller.clone(), database)))
    }

    let _ = futures::future::join_all(futures).await;
}
