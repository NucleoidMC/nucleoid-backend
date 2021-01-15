use std::collections::HashMap;
use std::time::SystemTime;

use async_trait::async_trait;
use log::error;
use tokio_postgres::{Client, Statement};
use xtra::prelude::*;

use crate::{DatabaseConfig, TokioGlobal};
use crate::controller::*;
use crate::model::*;

pub async fn run(controller: Address<Controller>, config: DatabaseConfig) {
    let (client, connection) = tokio_postgres::connect(
        &format!("host={} port={} user={} password={} dbname={}", config.host, config.port, config.user, config.password, config.database),
        tokio_postgres::NoTls,
    ).await.expect("failed to open connection to database");

    let database = DatabaseClient { client, config, channels: HashMap::new() };
    let database = database.create(None).spawn(&mut TokioGlobal);

    controller.do_send_async(RegisterDatabaseClient { client: database }).await
        .expect("controller disconnected");

    if let Err(err) = connection.await {
        error!("database connection error: {}", err);
    }
}

pub struct DatabaseClient {
    client: Client,
    config: DatabaseConfig,
    channels: HashMap<String, ChannelDatabase>,
}

impl DatabaseClient {
    async fn get_or_open_channel<'a>(
        channels: &'a mut HashMap<String, ChannelDatabase>,
        client: &mut Client,
        channel: String
    ) -> Result<&'a mut ChannelDatabase> {
        use std::collections::hash_map::Entry::*;
        match channels.entry(channel) {
            Occupied(occupied) => Ok(occupied.into_mut()),
            Vacant(vacant) => {
                let key = vacant.key().clone();
                let database = ChannelDatabase::open(client, key).await?;
                Ok(vacant.insert(database))
            }
        }
    }
}

impl Actor for DatabaseClient {}

pub struct WriteStatus {
    pub channel: String,
    pub time: SystemTime,
    pub status: ServerStatus,
}

impl Message for WriteStatus {
    type Result = ();
}

pub struct WritePerformance {
    pub channel: String,
    pub time: SystemTime,
    pub performance: ServerPerformance,
}

impl Message for WritePerformance {
    type Result = ();
}

#[async_trait]
impl Handler<WriteStatus> for DatabaseClient {
    async fn handle(&mut self, message: WriteStatus, _ctx: &mut Context<Self>) {
        let channel = DatabaseClient::get_or_open_channel(&mut self.channels, &mut self.client, message.channel).await
            .expect("failed to open database for channel");

        if let Err(err) = channel.write_status(&mut self.client, message.time, message.status).await {
            error!("failed to write status to database: {:?}", err);
        }
    }
}

#[async_trait]
impl Handler<WritePerformance> for DatabaseClient {
    async fn handle(&mut self, message: WritePerformance, _ctx: &mut Context<Self>) {
        let channel = DatabaseClient::get_or_open_channel(&mut self.channels, &mut self.client, message.channel).await
            .expect("failed to open database for channel");

        if let Err(err) = channel.write_performance(&mut self.client, message.time, message.performance).await {
            error!("failed to write status to database: {:?}", err);
        }
    }
}

struct ChannelDatabase {
    add_status: Statement,
    add_performance: Statement,
}

impl ChannelDatabase {
    async fn open(client: &mut Client, channel: String) -> Result<ChannelDatabase> {
        let status_table = format!("{}_server_status", channel);
        let performance_table = format!("{}_server_performance", channel);

        let create_status_table = format!(r#"
            CREATE TABLE IF NOT EXISTS {} (
                time TIMESTAMP WITHOUT TIME ZONE NOT NULL PRIMARY KEY,
                player_count SMALLINT NOT NULL,
                game_count SMALLINT NOT NULL,

                UNIQUE(time)
            )
        "#, status_table);

        let create_performance_table = format!(r#"
            CREATE TABLE IF NOT EXISTS {} (
                time TIMESTAMP WITHOUT TIME ZONE NOT NULL PRIMARY KEY,
                average_tick_ms SMALLINT NOT NULL,
                tps SMALLINT NOT NULL,
                dimensions SMALLINT NOT NULL,
                entities INT NOT NULL,
                chunks INT NOT NULL,
                used_memory BIGINT NOT NULL,
                total_memory BIGINT NOT NULL,

                UNIQUE(time)
            )
        "#, performance_table);

        let create_status_table = client.prepare(&create_status_table).await?;
        client.execute(&create_status_table, &[]).await?;

        let create_performance_table = client.prepare(&create_performance_table).await?;
        client.execute(&create_performance_table, &[]).await?;

        let add_status = format!(r#"
            INSERT INTO {} (time, player_count, game_count) VALUES ($1, $2, $3)
        "#, status_table);
        let add_status = client.prepare(&add_status).await?;

        let add_performance = format!(r#"
            INSERT INTO {} (time, average_tick_ms, tps, dimensions, entities, chunks, used_memory, total_memory) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#, performance_table);
        let add_performance = client.prepare(&add_performance).await?;

        Ok(ChannelDatabase {
            add_status,
            add_performance
        })
    }

    async fn write_status(&self, client: &mut Client, time: SystemTime, status: ServerStatus) -> Result<()> {
        let player_count = status.players.len() as i16;
        let game_count = status.games.len() as i16;
        client.execute(&self.add_status, &[&time, &player_count, &game_count]).await?;
        Ok(())
    }

    async fn write_performance(&self, client: &mut Client, time: SystemTime, performance: ServerPerformance) -> Result<()> {
        let average_tick_ms = performance.average_tick_ms as i16;
        let tps = performance.tps as i16;
        let dimensions = performance.dimensions as i16;
        let entities = performance.entities as i32;
        let chunks = performance.chunks as i32;
        let used_memory = performance.used_memory as i64;
        let total_memory = performance.total_memory as i64;

        client.execute(&self.add_performance, &[&time, &average_tick_ms, &tps, &dimensions, &entities, &chunks, &used_memory, &total_memory]).await?;
        Ok(())
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("postgres error")]
    Postgres(#[from] tokio_postgres::Error),
}
