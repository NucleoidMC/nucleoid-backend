use std::collections::HashMap;
use std::time::SystemTime;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use log::error;
use xtra::prelude::*;

use crate::controller::*;
use crate::model::*;
use crate::{DatabaseConfig, TokioGlobal};

pub async fn run(controller: Address<Controller>, pool: Pool, config: DatabaseConfig) {
    let database = DatabaseClient {
        pool,
        _config: config,
        channels: HashMap::new(),
    };
    let database = database.create(None).spawn(&mut TokioGlobal);

    controller
        .do_send_async(RegisterDatabaseClient { client: database })
        .await
        .expect("controller disconnected");
}

pub struct DatabaseClient {
    pool: Pool,
    _config: DatabaseConfig,
    channels: HashMap<String, ChannelDatabase>,
}

impl DatabaseClient {
    async fn get_or_open_channel(
        channels: &mut HashMap<String, ChannelDatabase>,
        pool: Pool,
        channel: String,
    ) -> Result<&mut ChannelDatabase> {
        use std::collections::hash_map::Entry::*;
        match channels.entry(channel) {
            Occupied(occupied) => Ok(occupied.into_mut()),
            Vacant(vacant) => {
                let key = vacant.key().clone();
                let database = ChannelDatabase::open(pool, key).await?;
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

pub struct GetPostgresPool;

impl Message for GetPostgresPool {
    type Result = Pool;
}

#[async_trait]
impl Handler<WriteStatus> for DatabaseClient {
    async fn handle(&mut self, message: WriteStatus, _ctx: &mut Context<Self>) {
        let channel = DatabaseClient::get_or_open_channel(
            &mut self.channels,
            self.pool.clone(),
            message.channel,
        )
        .await
        .expect("failed to open database for channel");

        if let Err(err) = channel
            .write_status(self.pool.clone(), message.time, message.status)
            .await
        {
            error!("failed to write status to database: {:?}", err);
        }
    }
}

#[async_trait]
impl Handler<WritePerformance> for DatabaseClient {
    async fn handle(&mut self, message: WritePerformance, _ctx: &mut Context<Self>) {
        let channel = DatabaseClient::get_or_open_channel(
            &mut self.channels,
            self.pool.clone(),
            message.channel,
        )
        .await
        .expect("failed to open database for channel");

        if let Err(err) = channel
            .write_performance(self.pool.clone(), message.time, message.performance)
            .await
        {
            error!("failed to write status to database: {:?}", err);
        }
    }
}

#[async_trait]
impl Handler<GetPostgresPool> for DatabaseClient {
    async fn handle(
        &mut self,
        _message: GetPostgresPool,
        _ctx: &mut Context<Self>,
    ) -> <GetPostgresPool as Message>::Result {
        self.pool.clone()
    }
}

struct ChannelDatabase {
    add_status: String,
    add_performance: String,
}

impl ChannelDatabase {
    async fn open(pool: Pool, channel: String) -> Result<ChannelDatabase> {
        let status_table = format!("{}_server_status", channel);
        let performance_table = format!("{}_server_performance", channel);

        let create_status_table = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                time TIMESTAMP WITHOUT TIME ZONE NOT NULL PRIMARY KEY,
                player_count SMALLINT NOT NULL,
                game_count SMALLINT NOT NULL,

                UNIQUE(time)
            )
        "#,
            status_table
        );

        let create_performance_table = format!(
            r#"
            CREATE TABLE IF NOT EXISTS {} (
                time TIMESTAMP WITHOUT TIME ZONE NOT NULL PRIMARY KEY,
                average_tick_ms REAL NOT NULL,
                tps SMALLINT NOT NULL,
                dimensions SMALLINT NOT NULL,
                entities INT NOT NULL,
                chunks INT NOT NULL,
                used_memory BIGINT NOT NULL,
                total_memory BIGINT NOT NULL,

                UNIQUE(time)
            )
        "#,
            performance_table
        );

        let client = pool.get().await?;

        let create_status_table = client.prepare(&create_status_table).await?;
        client.execute(&create_status_table, &[]).await?;

        let create_performance_table = client.prepare(&create_performance_table).await?;
        client.execute(&create_performance_table, &[]).await?;

        let add_status = format!(
            r#"
            INSERT INTO {} (time, player_count, game_count) VALUES ($1, $2, $3)
        "#,
            status_table
        );

        let add_performance = format!(
            r#"
            INSERT INTO {} (time, average_tick_ms, tps, dimensions, entities, chunks, used_memory, total_memory) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        "#,
            performance_table
        );

        Ok(ChannelDatabase {
            add_status,
            add_performance,
        })
    }

    async fn write_status(&self, pool: Pool, time: SystemTime, status: ServerStatus) -> Result<()> {
        let client = pool.get().await?;
        let player_count = status.players.len() as i16;
        let game_count = status.games.len() as i16;
        let statement = client.prepare_cached(&self.add_status).await?;
        client
            .execute(&statement, &[&time, &player_count, &game_count])
            .await?;
        Ok(())
    }

    async fn write_performance(
        &self,
        pool: Pool,
        time: SystemTime,
        performance: ServerPerformance,
    ) -> Result<()> {
        let client = pool.get().await?;
        let average_tick_ms = performance.average_tick_ms as f32;
        let tps = performance.tps as i16;
        let dimensions = performance.dimensions as i16;
        let entities = performance.entities as i32;
        let chunks = performance.chunks as i32;
        let used_memory = performance.used_memory as i64;
        let total_memory = performance.total_memory as i64;

        let statement = client.prepare_cached(&self.add_performance).await?;
        client
            .execute(
                &statement,
                &[
                    &time,
                    &average_tick_ms,
                    &tps,
                    &dimensions,
                    &entities,
                    &chunks,
                    &used_memory,
                    &total_memory,
                ],
            )
            .await?;
        Ok(())
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("postgres error")]
    Postgres(#[from] tokio_postgres::Error),
    #[error("pool error")]
    PostgresPool(#[from] deadpool_postgres::PoolError),
}
