use deadpool_postgres::{Pool, Runtime};
use std::future::Future;
use tokio_postgres::NoTls;

use xtra::prelude::*;
use xtra::spawn::Spawner;

pub use config::*;
pub use controller::*;
pub use persistent::*;

mod config;
mod controller;
mod database;
mod discord;
mod integrations;
mod model;
mod mojang_api;
mod persistent;
mod statistics;
mod web;

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
    let controller = Controller::new(config.clone())
        .await
        .create(None)
        .spawn(&mut TokioGlobal);

    let mut futures = Vec::with_capacity(5);

    if let Some(integrations) = config.integrations {
        futures.push(tokio::spawn(integrations::run(
            controller.clone(),
            integrations,
        )));
    }

    if let Some(web) = config.web_server {
        futures.push(tokio::spawn(web::run(controller.clone(), web)));
    }

    if let Some(discord) = config.discord {
        futures.push(tokio::spawn(discord::run(controller.clone(), discord)));
    }

    if let Some(database) = config.database {
        let postgres_pool = setup_postgres(database.clone()).await;

        futures.push(tokio::spawn(database::run(
            controller.clone(),
            postgres_pool.clone(),
            database,
        )));

        if let Some(statistics) = config.statistics {
            futures.push(tokio::spawn(statistics::run(
                controller.clone(),
                statistics,
                postgres_pool.clone(),
            )));
        }
    }

    let _ = futures::future::join_all(futures).await;
}

async fn setup_postgres(config: DatabaseConfig) -> Pool {
    let mut db_config = deadpool_postgres::Config::new();
    db_config.host = Some(config.host.clone());
    db_config.port = Some(config.port);
    db_config.user = Some(config.user.clone());
    db_config.password = Some(config.password.clone());
    db_config.dbname = Some(config.database.clone());
    let pool = db_config
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .expect("failed to create database pool");
    pool
}
