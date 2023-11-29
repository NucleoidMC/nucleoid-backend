use clickhouse_rs::Pool;
use futures::StreamExt;
use nucleoid_leaderboards::model::{LeaderboardDefinition, LeaderboardQuery, ValueFormat};
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

use crate::statistics::database::StatisticsDatabaseResult;
use crate::statistics::leaderboards::{LeaderboardEntry, LeaderboardGenerator, LeaderboardValue};

pub const CREATE_LEADERBOARDS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS leaderboard_rankings(
    player_id uuid NOT NULL,
    leaderboard_id text NOT NULL,
    ranking bigint NOT NULL,
    value double precision NOT NULL,
    PRIMARY KEY (player_id, leaderboard_id)
);
"#;

pub async fn setup_leaderboard_tables(
    client: &deadpool_postgres::Object,
) -> StatisticsDatabaseResult<()> {
    client.execute(CREATE_LEADERBOARDS_TABLE, &[]).await?;

    Ok(())
}

#[derive(Serialize)]
pub struct LeaderboardMetadata {
    pub id: String,
    pub value_format: ValueFormat,
}

pub struct LeaderboardsDatabase {
    postgres_pool: deadpool_postgres::Pool,
    clickhouse_pool: clickhouse_rs::Pool,
    generator: LeaderboardGenerator,
}

impl LeaderboardsDatabase {
    pub async fn new(
        postgres_pool: deadpool_postgres::Pool,
        clickhouse_pool: Pool,
        leaderboards: Vec<LeaderboardDefinition>,
    ) -> StatisticsDatabaseResult<Self> {
        let client = postgres_pool.get().await?;
        setup_leaderboard_tables(&client).await?;

        Ok(Self {
            postgres_pool,
            clickhouse_pool,
            generator: LeaderboardGenerator::new(leaderboards),
        })
    }

    pub async fn update_all_leaderboards(&self) -> StatisticsDatabaseResult<()> {
        let client = self.postgres_pool.get().await?;
        let mut handle = self.clickhouse_pool.get_handle().await?;
        let statement = client
            .prepare_cached(
                r#"
        INSERT INTO leaderboard_rankings (player_id, leaderboard_id, ranking, value)
        VALUES($1, $2, $3, $4)
        ON CONFLICT (player_id, leaderboard_id)
        DO UPDATE SET ranking = $3, value = $4
        "#,
            )
            .await?;

        for leaderboard in self.generator.list_all_leaderboards() {
            let entries = self
                .generator
                .build_leaderboard(&mut handle, &leaderboard)
                .await?;
            if let Some(mut entries) = entries {
                let mut rank = 1_i64;
                while let Some(entry) = entries.next().await {
                    let entry: LeaderboardValue = entry?;
                    client
                        .execute(
                            &statement,
                            &[&entry.player_id, &leaderboard, &rank, &entry.value],
                        )
                        .await?;
                    rank += 1;
                }
            }
        }

        Ok(())
    }

    pub async fn get_leaderboard(
        &self,
        id: &str,
    ) -> StatisticsDatabaseResult<Option<Vec<LeaderboardEntry>>> {
        let client = self.postgres_pool.get().await?;
        let statement = client
            .prepare_cached(
                r#"
        SELECT player_id, ranking, value
        FROM leaderboard_rankings
        WHERE leaderboard_id = $1
        ORDER BY ranking ASC
        LIMIT $2
        "#,
            )
            .await?;
        let res = client.query(&statement, &[&id, &10_i64]).await?;
        let leaderboard = res
            .iter()
            .map(|row| {
                let player = row.get::<_, Uuid>("player_id");
                let ranking = row.get::<_, i64>("ranking");
                let value = row.get::<_, f64>("value");
                LeaderboardEntry {
                    player,
                    ranking,
                    value,
                }
            })
            .collect::<Vec<_>>();
        Ok(if leaderboard.is_empty() {
            None
        } else {
            Some(leaderboard)
        })
    }

    pub async fn get_leaderboard_metadata(
        &self,
        id: &str,
    ) -> StatisticsDatabaseResult<Option<LeaderboardMetadata>> {
        let metadata = self.generator.definitions.get(id).map(|(definition, _sql)| {
            let value_format = match definition.query {
                LeaderboardQuery::Sql { value_format, .. } => value_format,
                LeaderboardQuery::Statistic { value_format, .. } => value_format,
            };
            LeaderboardMetadata {
                id: id.to_owned(),
                value_format,
            }
        });
        Ok(metadata)
    }

    pub async fn get_player_rankings(
        &self,
        player: &Uuid,
    ) -> StatisticsDatabaseResult<Option<HashMap<String, (i64, f64)>>> {
        let client = self.postgres_pool.get().await?;
        let statement = client
            .prepare_cached(
                r#"
        SELECT leaderboard_id, ranking, value
        FROM leaderboard_rankings
        WHERE player_id = $1
        "#,
            )
            .await?;
        let res = client.query(&statement, &[player]).await?;
        let mut rankings = HashMap::new();
        for row in res {
            let leaderboard_id = row.get::<_, String>("leaderboard_id");
            let ranking = row.get::<_, i64>("ranking");
            let value = row.get::<_, f64>("value");
            rankings.insert(leaderboard_id, (ranking, value));
        }

        Ok(if rankings.is_empty() {
            None
        } else {
            Some(rankings)
        })
    }

    pub fn list_all_leaderboards(&self) -> Vec<String> {
        self.generator.list_all_leaderboards()
    }
}
