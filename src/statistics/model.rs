use std::collections::HashMap;

use chrono::DateTime;
use chrono::Utc;
use chrono_tz::Tz;
use clickhouse_rs::Pool;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CREATE_GAMES_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS games(
    game_id         UUID DEFAULT generateUUIDv4(),
    namespace       String,
    player_count    UInt32,
    server          String,
    date_played     DateTime
) Engine=MergeTree() PRIMARY KEY game_id
"#;

pub const CREATE_PLAYER_STATS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS player_statistics(
    statistic_id    UUID DEFAULT generateUUIDv4(),
    game_id         UUID,
    player_id       UUID,
    namespace       String,
    key             String,
    value           Float64,
    type            String
) Engine=MergeTree() PRIMARY KEY statistic_id
"#;

pub const CREATE_GLOBAL_STATS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS global_statistics(
    statistic_id    UUID DEFAULT generateUUIDv4(),
    game_id         UUID,
    namespace       String,
    key             String,
    value           Float64,
    type            String
) Engine=MergeTree() PRIMARY KEY statistic_id
"#;

pub async fn initialise_database(db: &Pool) -> Result<(), clickhouse_rs::errors::Error> {
    let mut client = db.get_handle().await?;

    // See if we can connect
    client.ping().await?;

    client.execute(CREATE_GAMES_TABLE).await?;
    client.execute(CREATE_PLAYER_STATS_TABLE).await?;
    client.execute(CREATE_GLOBAL_STATS_TABLE).await?;
    Ok(())
}

pub type PlayerStatsResponse = HashMap<String, HashMap<String, f64>>;
pub type PlayerStatsBundle = HashMap<Uuid, HashMap<String, UploadStat>>;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GameStatsBundle {
    pub namespace: String,
    pub stats: StatsBundle,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct StatsBundle {
    pub global: Option<HashMap<String, UploadStat>>,
    pub players: PlayerStatsBundle,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RecentGame {
    pub id: Uuid,
    pub namespace: String,
    pub players: Vec<Uuid>,
    pub server: String,
    pub date_played: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum UploadStat {
    IntTotal(i32),
    IntMin(i32),
    IntMax(i32),
    IntRollingAverage(i32),
    FloatTotal(f64),
    FloatMin(f64),
    FloatMax(f64),
    FloatRollingAverage(f64),
}

impl UploadStat {
    pub fn get_type(&self) -> &str {
        match self {
            UploadStat::IntTotal(_) => "int_total",
            UploadStat::IntMin(_) => "int_min",
            UploadStat::IntMax(_) => "int_max",
            UploadStat::IntRollingAverage(_) => "int_rolling_average",
            UploadStat::FloatTotal(_) => "float_total",
            UploadStat::FloatMin(_) => "float_min",
            UploadStat::FloatMax(_) => "float_max",
            UploadStat::FloatRollingAverage(_) => "float_rolling_average",
        }
    }
}

impl Into<f64> for UploadStat {
    fn into(self) -> f64 {
        // I hate this but until anyone else has a better idea then this will stay
        match self {
            UploadStat::FloatTotal(v) |
            UploadStat::FloatMin(v) |
            UploadStat::FloatMax(v) |
            UploadStat::FloatRollingAverage(v) => v,

            UploadStat::IntTotal(v) |
            UploadStat::IntMin(v) |
            UploadStat::IntMax(v) |
            UploadStat::IntRollingAverage(v) => v as f64,
        }
    }
}

#[derive(Serialize)]
pub struct StatisticsStats {
    pub unique_players: u64,
    pub games_played: u64,
    pub entries: StatisticCounts<u64>,
    pub grand_total: StatisticCounts<f64>,
}

#[derive(Serialize)]
pub struct StatisticCounts<T> {
    pub player: T,
    pub global: T,
    pub total: T,
}

#[derive(Serialize)]
pub struct Datapoint {
    pub date: DateTime<Tz>,
    pub value: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataQueryType {
    GamesByDay,
    GamesByMonth,
}
