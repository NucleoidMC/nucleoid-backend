use serde::{Serialize, Deserialize};
use uuid::Uuid;
use bson::{Document, doc};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerProfile {
    #[serde(with = "bson::serde_helpers::uuid_as_binary")]
    pub uuid: Uuid,
    pub username: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct PlayerProfileResponse {
    pub uuid: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

impl From<PlayerProfile> for PlayerProfileResponse {
    fn from(p: PlayerProfile) -> Self {
        Self {
            uuid: p.uuid,
            username: p.username,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PlayerGameStats {
    #[serde(with = "bson::serde_helpers::uuid_as_binary")]
    pub uuid: Uuid,
    pub namespace: String,
    pub stats: HashMap<String, GameStat>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GlobalGameStats {
    pub namespace: String,
    pub stats: HashMap<String, GameStat>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum GameStat {
    IntTotal(i32),
    IntRollingAverage {
        total: i32,
        count: i32,
    },
    FloatTotal(f64),
    FloatRollingAverage {
        total: f64,
        count: i32,
    },
}

impl Into<f64> for GameStat {
    fn into(self) -> f64 {
        match self {
            GameStat::IntTotal(v) => v as f64,
            GameStat::IntRollingAverage { total, count } => (total as f64) / (count as f64),
            GameStat::FloatTotal(v) => v,
            GameStat::FloatRollingAverage { total, count } => total / (count as f64),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GlobalStatsBundle {
    pub namespace: String,
    pub stats: HashMap<String, GameStat>,
}

pub type PlayerStatsResponse = HashMap<String, HashMap<String, f64>>;
pub type PlayerStatsBundle = HashMap<Uuid, HashMap<String, UploadStat>>;

#[derive(Serialize, Deserialize)]
pub struct GameStatsBundle {
    pub server_name: String,
    pub namespace: String,
    pub stats: StatsBundle,
}

#[derive(Serialize, Deserialize)]
pub struct StatsBundle {
    pub global: Option<HashMap<String, UploadStat>>,
    pub players: PlayerStatsBundle,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum UploadStat {
    IntTotal(i32),
    IntRollingAverage(i32),
    FloatTotal(f64),
    FloatRollingAverage(f64),
}

impl UploadStat {
    /// Generate a BSON document for increasing this value.
    pub fn create_increment_operation(&self, id: &str) -> Document {
        let value_key = format!("stats.{}.value", id);
        let type_key = format!("stats.{}.type", id);
        let total_key = format!("{}.total", value_key);
        let count_key = format!("{}.count", value_key);

        match self {
            UploadStat::IntTotal(value) => doc! {
                "$inc": { value_key: value },
                "$set": { type_key: "int_total" }
            },
            UploadStat::IntRollingAverage(value) => doc! {
                "$inc": { total_key: value, count_key: 1 },
                "$set": { type_key: "int_rolling_average" }
            },
            UploadStat::FloatTotal(value) => doc! {
                "$inc": { value_key: value },
                "$set": { type_key: "float_total" }
            },
            UploadStat::FloatRollingAverage(value) => doc! {
                "$inc": { total_key: value, count_key: 1 },
                "$set": { type_key: "float_rolling_average" }
            },
        }
    }
}
