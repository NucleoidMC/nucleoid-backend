use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use clickhouse_rs::{Block, Pool, row};
use log::warn;
use nucleoid_leaderboards::model::LeaderboardDefinition;
use uuid::Uuid;
use xtra::{Actor, Address, Context, Handler, Message};

use crate::{Controller, StatisticsConfig};
use crate::statistics::leaderboards::{LeaderboardEntry, LeaderboardGenerator};
use crate::statistics::model::{GameStatsBundle, initialise_database, PlayerStatsResponse, RecentGame, StatisticCounts, StatisticsStats};

pub struct StatisticDatabaseController {
    _controller: Address<Controller>,
    pool: Pool,
    _config: StatisticsConfig,
    leaderboards: LeaderboardGenerator,
}

impl StatisticDatabaseController {
    pub async fn connect(controller: &Address<Controller>, config: &StatisticsConfig, leaderboards: Vec<LeaderboardDefinition>) -> StatisticsDatabaseResult<Self> {
        let pool = Pool::new(config.database_url.clone());
        let handler = Self {
            _controller: controller.clone(),
            pool: pool.clone(),
            _config: config.clone(),
            leaderboards: LeaderboardGenerator::new(pool.clone(), leaderboards),
        };

        initialise_database(&handler.pool).await?;

        Ok(handler)
    }

    async fn get_player_stats(&self, player_id: &Uuid, namespace: &Option<String>) -> StatisticsDatabaseResult<Option<PlayerStatsResponse>> {
        let mut handle = self.pool.get_handle().await?;

        let cond = match namespace {
            Some(namespace) => format!("player_id = '{}' AND namespace = '{}'", player_id, namespace),
            None => format!("player_id = '{}'", player_id),
        };

        let sql = format!(
            r#"
            SELECT
                namespace,
                key,
                SUM(value)
            FROM player_statistics
            WHERE
                {}
            GROUP BY
                namespace,
                key
            ORDER BY
                key ASC
            "#, cond);

        let block = handle.query(sql).fetch_all().await?;

        let mut result = HashMap::new();
        for row in block.rows() {
            let namespace: String = row.get("namespace")?;
            let key: String = row.get("key")?;
            let value: f64 = row.get("sum(value)")?;
            if !result.contains_key(&namespace) {
                result.insert(namespace.clone(), HashMap::new());
            }
            result.get_mut(&namespace).unwrap().insert(key, value);
        }

        if result.is_empty() {
            Ok(None)
        } else {
            Ok(Some(result))
        }
    }

    async fn get_recent_games(&self, limit: u32, player_id: Option<Uuid>) -> StatisticsDatabaseResult<Vec<RecentGame>> {
        let mut handle = self.pool.get_handle().await?;

        let sql = match player_id {
            Some(player_id) => format!(r#"
            SELECT
                game_id,
                namespace,
                player_count,
                server,
                date_played
            FROM
                player_statistics
            INNER JOIN games
                ON player_statistics.game_id=games.game_id
            WHERE
                player_id = '{}'
            GROUP BY
                game_id,
                namespace,
                player_count,
                server,
                date_played
            ORDER BY date_played DESC
            LIMIT {}
            "#, player_id, limit),
            None => format!(r#"
                SELECT *
                FROM games
                ORDER BY date_played DESC
                LIMIT {}
                "#, limit),
        };

        let games_res = handle.query(sql).fetch_all().await?;

        if games_res.is_empty() {
            return Ok(Vec::with_capacity(0));
        }

        let mut games = Vec::new();

        for row in games_res.rows() {
            let game_id: Uuid = row.get("game_id")?;
            let namespace: String = row.get("namespace")?;
            let player_count: u32 = row.get("player_count")?;
            let server: String = row.get("server")?;
            let date_played: DateTime<Tz> = row.get("date_played")?;

            let players_sql = format!(r#"
            SELECT player_id
            FROM player_statistics
            WHERE game_id = '{}'
            GROUP BY player_id
            "#, game_id);

            let players_res = handle.query(players_sql).fetch_all().await?;
            let mut players = Vec::with_capacity(player_count as usize);
            for player_id in players_res.rows().map(|row| row.get::<Uuid, _>("player_id")) {
                players.push(player_id?);
            }

            games.push(RecentGame {
                id: game_id,
                namespace,
                players,
                server,
                date_played: date_played.with_timezone(&Utc),
            });
        }

        Ok(games)
    }

    async fn get_game_stats(&self, game_id: &Uuid) -> StatisticsDatabaseResult<Option<HashMap<Uuid, PlayerStatsResponse>>> {
        let mut handle = self.pool.get_handle().await?;

        let game_sql = format!("SELECT game_id FROM games WHERE game_id = '{}'", game_id);

        if handle.query(game_sql).fetch_all().await?.is_empty() {
            return Ok(None);
        }

        // This should be safe, as although a uuid is potentially-untrusted user input,
        // they are strictly formed and so no escape characters can be used to break out
        // of the sql string and manipulate the query.
        let players_sql = format!(r#"
            SELECT player_id, namespace, key, value, type
                FROM player_statistics
                WHERE game_id = '{}'"#, game_id);
        let global_sql = format!(r#"
            SELECT namespace, key, value, type
                FROM global_statistics
                WHERE game_id = '{}'"#, game_id);

        let players_res = handle.query(players_sql).fetch_all().await?;
        let global_res = handle.query(global_sql).fetch_all().await?;

        if players_res.is_empty() && global_res.is_empty() {
            return Ok(None);
        }

        let mut players = HashMap::new();

        for row in players_res.rows() {
            let player_id: Uuid = row.get("player_id")?;
            let namespace: String = row.get("namespace")?;
            let key: String = row.get("key")?;
            let value: f64 = row.get("value")?;
            // let stat_type: String = row.get("type")?;
            if !players.contains_key(&player_id) {
                players.insert(player_id, HashMap::new());
            }
            let player_stats = players.get_mut(&player_id).unwrap();
            if !player_stats.contains_key(&namespace) {
                player_stats.insert(namespace.clone(), HashMap::new());
            }
            let stats = player_stats.get_mut(&namespace).unwrap();
            stats.insert(key, value);
        }

        let global_player_id = Uuid::nil();
        for row in global_res.rows() {
            let namespace: String = row.get("namespace")?;
            let key: String = row.get("key")?;
            let value: f64 = row.get("value")?;
            // let stat_type: String = row.get("type")?;
            if !players.contains_key(&global_player_id) {
                players.insert(global_player_id, HashMap::new());
            }
            let player_stats = players.get_mut(&global_player_id).unwrap();
            if !player_stats.contains_key(&namespace) {
                player_stats.insert(namespace.clone(), HashMap::new());
            }
            let stats = player_stats.get_mut(&namespace).unwrap();
            stats.insert(key, value);
        }

        Ok(Some(players))
    }

    async fn upload_stats_bundle(&self, game_id: Uuid, server: &String, bundle: GameStatsBundle) -> StatisticsDatabaseResult<Uuid> {
        let mut handle = self.pool.get_handle().await?;

        // Steps to insert a whole stats bundle
        {
            let date_played = Utc::now().with_timezone(&Tz::GMT);

            // 1. Insert a row into the games table and record the allocated ID
            let mut block = Block::with_capacity(1);
            block.push(row! {
                game_id: game_id,
                namespace: bundle.namespace.clone(),
                player_count: bundle.stats.players.len() as u32,
                server: server.clone(),
                date_played: date_played,
            })?;

            handle.insert("games", block).await?;
        }

        {
            // 2. Insert all player statistics into the player_statistics table
            let mut block = Block::with_capacity(bundle.stats.players.len());
            for (player, stats) in bundle.stats.players {
                for (key, stat) in stats {
                    let value: f64 = stat.clone().into();
                    block.push(row! {
                        game_id: game_id,
                        player_id: player,
                        namespace: bundle.namespace.clone(),
                        key: key.clone(),
                        value: value,
                        type: stat.clone().get_type(),
                    })?;
                }
            }
            handle.insert("player_statistics", block).await?;
        }

        // 3. Insert all global statistics into the global_statistics table
        if let Some(global) = bundle.stats.global {
            let mut block = Block::with_capacity(global.len());
            for (key, stat) in global {
                let value: f64 = stat.clone().into();
                block.push(row! {
                    game_id: game_id,
                    namespace: bundle.namespace.clone(),
                    key: key.clone(),
                    value: value,
                    type: stat.get_type(),
                })?;
            }
            handle.insert("global_statistics", block).await?;
        }

        Ok(game_id)
    }

    async fn get_statistics_stats(&self) -> StatisticsDatabaseResult<StatisticsStats> {
        let mut handle = self.pool.get_handle().await?;
        let player_results = handle.query(r#"
        SELECT COUNT(DISTINCT player_id) AS unique_players,
            COUNT(*) AS total_entries,
            SUM(value) AS grand_total
        FROM player_statistics"#).fetch_all().await?;
        let game_results = handle.query("SELECT COUNT(*) AS games_played FROM games").fetch_all().await?;
        let global_results = handle.query("SELECT COUNT(*) AS total_entries, SUM(value) as grand_total FROM global_statistics").fetch_all().await?;

        let unique_players = player_results.get(0, "unique_players")?;
        let games_played = game_results.get(0, "games_played")?;
        let player_entries = player_results.get(0, "total_entries")?;
        let global_entries = global_results.get(0, "total_entries")?;
        let total_entries = player_entries + global_entries;
        let player_total = player_results.get(0, "grand_total")?;
        let global_total = global_results.get(0, "grand_total")?;
        let grand_total = player_total + global_total;
        Ok(StatisticsStats {
            unique_players,
            games_played,
            entries: StatisticCounts {
                player: player_entries,
                global: global_entries,
                total: total_entries,
            },
            grand_total: StatisticCounts {
                player: player_total,
                global: global_total,
                total: grand_total,
            },
        })
    }
}

impl Actor for StatisticDatabaseController {}

pub struct GetPlayerStats {
    pub uuid: Uuid,
    pub namespace: Option<String>,
}

impl Message for GetPlayerStats {
    type Result = StatisticsDatabaseResult<Option<PlayerStatsResponse>>;
}

#[async_trait]
impl Handler<GetPlayerStats> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetPlayerStats, _ctx: &mut Context<Self>) -> <GetPlayerStats as Message>::Result {
        self.get_player_stats(&message.uuid, &message.namespace).await
    }
}

pub struct GetGameStats(pub Uuid);

impl Message for GetGameStats {
    type Result = StatisticsDatabaseResult<Option<HashMap<Uuid, PlayerStatsResponse>>>;
}

#[async_trait]
impl Handler<GetGameStats> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetGameStats, _ctx: &mut Context<Self>) -> <GetGameStats as Message>::Result {
        self.get_game_stats(&message.0).await
    }
}

pub struct GetRecentGames {
    pub limit: u32,
    pub player_id: Option<Uuid>,
}

impl Message for GetRecentGames {
    type Result = StatisticsDatabaseResult<Vec<RecentGame>>;
}

#[async_trait]
impl Handler<GetRecentGames> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetRecentGames, _ctx: &mut Context<Self>) -> <GetRecentGames as Message>::Result {
        self.get_recent_games(message.limit, message.player_id).await
    }
}

#[derive(Debug)]
pub struct UploadStatsBundle {
    pub game_id: Uuid,
    pub server: String,
    pub bundle: GameStatsBundle,
}

impl Message for UploadStatsBundle {
    type Result = ();
}

#[async_trait]
impl Handler<UploadStatsBundle> for StatisticDatabaseController {
    async fn handle(&mut self, message: UploadStatsBundle, _ctx: &mut Context<Self>) -> <UploadStatsBundle as Message>::Result {
        if let Err(e) = self.upload_stats_bundle(
            message.game_id, &message.server.clone(), message.bundle.clone()
        ).await {
            warn!("Failed to upload stats bundle {:?}: {}", message, e);
        }
    }
}

pub struct GetStatisticsStats;

impl Message for GetStatisticsStats {
    type Result = StatisticsDatabaseResult<StatisticsStats>;
}

#[async_trait]
impl Handler<GetStatisticsStats> for StatisticDatabaseController {
    async fn handle(&mut self, _message: GetStatisticsStats, _ctx: &mut Context<Self>) -> <GetStatisticsStats as Message>::Result {
        self.get_statistics_stats().await
    }
}

pub struct GetLeaderboard(pub String);

impl Message for GetLeaderboard {
    type Result = StatisticsDatabaseResult<Option<Vec<LeaderboardEntry>>>;
}

#[async_trait]
impl Handler<GetLeaderboard> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetLeaderboard, _ctx: &mut Context<Self>) -> <GetLeaderboard as Message>::Result {
        self.leaderboards.build_leaderboard(&message.0).await
    }
}

#[derive(thiserror::Error, Debug)]
pub enum StatisticsDatabaseError {
    #[error("a database error occurred: {0}")]
    ClickHouseError(#[from] clickhouse_rs::errors::Error),
    #[error("unknown error")]
    UnknownError,
}

pub type StatisticsDatabaseResult<T> = Result<T, StatisticsDatabaseError>;
