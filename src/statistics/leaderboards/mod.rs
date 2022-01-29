pub mod database;

use std::collections::HashMap;

use futures::{Stream, StreamExt};
use nucleoid_leaderboards::model::{Aggregate, LeaderboardDefinition, LeaderboardQuery, Ranking, UnitConversion, ValueType};
use serde::Serialize;
use uuid::Uuid;

use crate::statistics::database::StatisticsDatabaseResult;

pub struct LeaderboardGenerator {
    definitions: HashMap<String, (LeaderboardDefinition, LeaderboardSql)>,
}

impl LeaderboardGenerator {
    pub fn new(definitions: Vec<LeaderboardDefinition>) -> Self {
        let mut definitions_map = HashMap::new();

        for definition in definitions {
            if definitions_map.contains_key(&definition.id) {
                log::warn!("Duplicate leaderboard definition for {}", definition.id);
            }
            let sql = build_sql(&definition);
            definitions_map.insert(definition.id.clone(), (definition, sql));
        }

        Self {
            definitions: definitions_map,
        }
    }

    pub async fn build_leaderboard<'a>(&self, handle: &'a mut clickhouse_rs::ClientHandle, id: &str) -> StatisticsDatabaseResult<Option<impl Stream<Item = StatisticsDatabaseResult<LeaderboardValue>> + 'a>> {
        let sql = match self.definitions.get(id) {
            Some(sql) => sql.1.clone(),
            None => return Ok(None),
        };

        let stream = handle.query(&sql.sql).stream().map(move |row|  {
            let row = row?;
            let player_id: Uuid = row.get(&*sql.player)?;
            let value = match sql.value_type {
                ValueType::Int => row.get::<i64, _>(&*sql.value)? as f64,
                ValueType::UInt => row.get::<u64, _>(&*sql.value)? as f64,
                ValueType::Float => row.get::<f64, _>(&*sql.value)?,
            };
            Ok(LeaderboardValue { player_id, value })
        });

        Ok(Some(stream))
    }

    pub fn list_all_leaderboards(&self) -> Vec<String> {
        self.definitions.keys().map(Clone::clone).collect::<Vec<_>>()
    }
}

#[derive(Serialize)]
pub struct LeaderboardValue {
    player_id: Uuid,
    value: f64,
}

#[derive(Serialize)]
pub struct LeaderboardEntry {
    player: Uuid,
    ranking: i64,
    value: f64,
}

fn build_sql(definition: &LeaderboardDefinition) -> LeaderboardSql {
    match &definition.query {
        LeaderboardQuery::Sql {
            query,
            player,
            value,
            value_type,
        } => LeaderboardSql {
            sql: query.clone(),
            player: player.clone(),
            value: value.clone(),
            value_type: value_type.clone(),
        },
        LeaderboardQuery::Statistic {
            namespace,
            key,
            aggregate,
            ranking,
            convert,
        } => LeaderboardSql {
            // TODO: Sanitize SQL here?
            sql: format!(
                r#"
                    SELECT
                        player_id, {aggregate}{convert} as value
                    FROM
                        player_statistics
                    WHERE
                        namespace = '{namespace}'
                        AND key = '{key}'
                    GROUP BY
                        player_id
                    ORDER BY value {ranking}
                    "#,
                namespace = namespace,
                key = key,
                aggregate = aggregate_sql(aggregate),
                convert = convert_sql(convert),
                ranking = ranking_sql(ranking),
            ),
            player: "player_id".to_string(),
            value: "value".to_string(),
            value_type: ValueType::Float,
        }
    }
}

fn aggregate_sql(aggregate: &Aggregate) -> &'static str {
    match aggregate {
        Aggregate::Total => "SUM(value)",
        Aggregate::Average => "SUM(value) / COUNT(value)",
        Aggregate::Minimum => "MIN(value)",
        Aggregate::Maximum => "MAX(value)",
    }
}

fn convert_sql(convert: &Option<UnitConversion>) -> &'static str {
    match convert {
        Some(UnitConversion::TicksToSeconds) => " / 20",
        None => "",
    }
}

fn ranking_sql(ranking: &Ranking) -> &'static str {
    match ranking {
        Ranking::Lowest => "ASC",
        Ranking::Highest => "DESC",
    }
}

#[derive(Clone)]
struct LeaderboardSql {
    sql: String,
    player: String,
    value: String,
    value_type: ValueType,
}
