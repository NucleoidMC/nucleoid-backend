use deadpool_postgres::{Pool, Runtime};
use tokio_postgres::NoTls;

use tracing_subscriber::prelude::*;
use xtra::prelude::*;

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

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "debug,serenity=info,rustls=info,h2=info,hyper=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::load();
    let controller = xtra::spawn_tokio(Controller::new(config.clone()).await, Mailbox::unbounded());

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
                postgres_pool,
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
    db_config.dbname = Some(config.database);

    db_config
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .expect("failed to create database pool")
}
