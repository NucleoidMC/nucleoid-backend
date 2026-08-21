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

pub fn spawn_tokio_named<A>(
    name: &str,
    actor: A,
    (address, mailbox): (crate::Address<A>, crate::Mailbox<A>),
) -> crate::Address<A>
where
    A: crate::Actor<Stop = ()>,
{
    tokio::task::Builder::new()
        .name(name)
        .spawn(xtra::run(mailbox, actor))
        .unwrap();

    address
}

#[tokio::main]
async fn main() {
    let console_layer = console_subscriber::spawn();
    tracing_subscriber::registry()
        .with(console_layer)
        .with(tracing_subscriber::fmt::layer().with_filter(
            tracing_subscriber::filter::EnvFilter::new(
                std::env::var("RUST_LOG").unwrap_or_else(|_| {
                    "debug,serenity=info,rustls=info,h2=info,hyper=info".into()
                }),
            ),
        ))
        .init();

    let config = config::load();
    let controller = spawn_tokio_named(
        "controller_actor",
        Controller::new(config.clone()).await,
        Mailbox::unbounded(),
    );

    let mut futures = Vec::with_capacity(5);

    if let Some(integrations) = config.integrations {
        futures.push(
            tokio::task::Builder::new()
                .name("integrations")
                .spawn(integrations::run(controller.clone(), integrations))
                .unwrap(),
        );
    }

    if let Some(web) = config.web_server {
        futures.push(
            tokio::task::Builder::new()
                .name("web_server")
                .spawn(web::run(controller.clone(), web))
                .unwrap(),
        );
    }

    if let Some(discord) = config.discord {
        futures.push(
            tokio::task::Builder::new()
                .name("discord")
                .spawn(discord::run(controller.clone(), discord))
                .unwrap(),
        );
    }

    if let Some(database) = config.database {
        let postgres_pool = setup_postgres(database.clone()).await;

        futures.push(
            tokio::task::Builder::new()
                .name("database")
                .spawn(database::run(
                    controller.clone(),
                    postgres_pool.clone(),
                    database,
                ))
                .unwrap(),
        );

        if let Some(statistics) = config.statistics {
            futures.push(
                tokio::task::Builder::new()
                    .name("statistics")
                    .spawn(statistics::run(
                        controller.clone(),
                        statistics,
                        postgres_pool,
                    ))
                    .unwrap(),
            );
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
