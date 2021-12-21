use clickhouse_rs::Pool;
use futures::StreamExt;
use nucleoid_leaderboards::model::LeaderboardDefinition;
use tokio_postgres::Statement;
use crate::statistics::database::StatisticsDatabaseResult;
use crate::statistics::leaderboards::{LeaderboardEntry, LeaderboardGenerator};

pub const CREATE_LEADERBOARDS_TABLE: &str = r#"
CREATE TABLE IF NOT EXISTS leaderboard_rankings(
    player_id uuid NOT NULL,
    leaderboard_id text NOT NULL,
    ranking bigint NOT NULL,
    UNIQUE (player_id, leaderboard_id)
);
"#;

pub async fn setup_leaderboard_tables(client: &deadpool_postgres::Object) -> StatisticsDatabaseResult<()> {
    client.execute(CREATE_LEADERBOARDS_TABLE, &[]).await?;

    Ok(())
}

pub struct LeaderboardsDatabase {
    postgres_pool: deadpool_postgres::Pool,
    clickhouse_pool: clickhouse_rs::Pool,
    upsert: Statement,
    generator: LeaderboardGenerator,
}

impl LeaderboardsDatabase {
    pub async fn new(postgres_pool: deadpool_postgres::Pool, clickhouse_pool: Pool, leaderboards: Vec<LeaderboardDefinition>) -> StatisticsDatabaseResult<Self> {
        let client = postgres_pool.get().await?;
        setup_leaderboard_tables(&client).await?;

        let upsert_statement = r#"
        INSERT INTO leaderboard_rankings (player_id, leaderboard_id, ranking)
        VALUES($1, $2, $3)
        ON CONFLICT (player_id, leaderboard_id)
        DO UPDATE SET ranking = $3
        "#;
        let upsert_statement = client.prepare(upsert_statement).await?;

        Ok(Self {
            postgres_pool,
            clickhouse_pool,
            upsert: upsert_statement,
            generator: LeaderboardGenerator::new(leaderboards),
        })
    }

    pub async fn update_all_leaderboards(&self) -> StatisticsDatabaseResult<()> {
        let client = self.postgres_pool.get().await?;
        let mut handle = self.clickhouse_pool.get_handle().await?;
        for leaderboard in self.generator.list_all_leaderboards() {
            let entries = self.generator.build_leaderboard(&mut handle, leaderboard).await?;
            if let Some(mut entries) = entries {
                let mut rank = 1_i64;
                while let Some(entry) = entries.next().await {
                    let entry: LeaderboardEntry = entry?;
                    client.execute(&self.upsert, &[&entry.player_id, leaderboard, &rank]).await?;
                    rank += 1;
                }
            }
        }

        Ok(())
    }
}
