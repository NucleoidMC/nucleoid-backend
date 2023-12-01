use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub struct NucleoidWrapped {
    clickhouse_pool: clickhouse_rs::Pool,
}

impl NucleoidWrapped {
    pub fn new(clickhouse_pool: clickhouse_rs::Pool) -> Self {
        Self { clickhouse_pool }
    }

    async fn played_count(&self, player: &Uuid) -> Result<u64, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle.query(format!(
            r#"
            SELECT
                COUNT(DISTINCT game_id) AS total
            FROM player_statistics
            INNER JOIN games ON player_statistics.game_id = games.game_id
            WHERE (player_id = '{player_id}') AND (games.date_played < '2023-12-01 00:00:00') AND (games.date_played > '2022-12-31 00:00:00')
            ORDER BY total DESC
            "#,
            // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
            player_id = player
        )).fetch_all().await?;
        if let Some(row) = results.rows().next() {
            Ok(row.get("total")?)
        } else {
            Ok(0)
        }
    }

    async fn top_games(
        &self,
        player: &Uuid,
    ) -> Result<Vec<PerGameStat>, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle.query(format!(
            r#"
            SELECT
                games.namespace as namespace,
                COUNT(DISTINCT game_id) AS total
            FROM player_statistics
            INNER JOIN games ON player_statistics.game_id = games.game_id
            WHERE (player_id = '{player_id}') AND (games.date_played < '2023-12-01 00:00:00') AND (games.date_played > '2022-12-31 00:00:00')
            GROUP BY games.namespace
            ORDER BY total DESC
            "#,
            // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
            player_id = player
        )).fetch_all().await?;

        let mut top_games = Vec::with_capacity(results.row_count());

        for row in results.rows() {
            let namespace: String = row.get("namespace")?;
            let total = row.get("total")?;
            top_games.push(PerGameStat { namespace, total });
        }

        Ok(top_games)
    }

    async fn days_played(&self, player: &Uuid) -> Result<u64, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle.query(format!(
            r#"
            SELECT
                COUNT(DISTINCT toDayOfYear(date_played)) AS total
            FROM player_statistics
            INNER JOIN games ON player_statistics.game_id = games.game_id
            WHERE (player_id = '{player_id}') AND (games.date_played < '2023-12-01 00:00:00') AND (games.date_played > '2022-12-31 00:00:00')
            ORDER BY total DESC
            "#,
            // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
            player_id = player
        )).fetch_all().await?;
        if let Some(row) = results.rows().next() {
            Ok(row.get("total")?)
        } else {
            Ok(0)
        }
    }

    async fn days_played_games(
        &self,
        player: &Uuid,
    ) -> Result<Vec<PerGameStat>, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle.query(format!(
            r#"
            SELECT
                games.namespace as namespace,
                COUNT(DISTINCT toDayOfYear(date_played)) AS total
            FROM player_statistics
            INNER JOIN games ON player_statistics.game_id = games.game_id
            WHERE (player_id = '{player_id}') AND (games.date_played < '2023-12-01 00:00:00') AND (games.date_played > '2022-12-31 00:00:00')
            GROUP BY games.namespace
            ORDER BY total DESC
            "#,
            // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
            player_id = player
        )).fetch_all().await?;

        let mut top_games = Vec::with_capacity(results.row_count());

        for row in results.rows() {
            let namespace: String = row.get("namespace")?;
            let total = row.get("total")?;
            top_games.push(PerGameStat { namespace, total });
        }

        Ok(top_games)
    }

    async fn most_players(&self, player: &Uuid) -> Result<u64, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle
            .query(format!(
                r#"
                SELECT
                    COUNT(DISTINCT player_id) as total
                FROM
                    (SELECT
                        game_id
                    FROM player_statistics
                    INNER JOIN games ON player_statistics.game_id = games.game_id
                    WHERE (player_id = '{player_id}')
                        AND (games.date_played < '2023-12-01 00:00:00')
                        AND (games.date_played > '2022-12-31 00:00:00')
                    GROUP BY game_id) AS games
                INNER JOIN player_statistics ON player_statistics.game_id = games.game_id
            "#,
                // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
                player_id = player
            ))
            .fetch_all()
            .await?;
        if let Some(row) = results.rows().next() {
            let mut total: u64 = row.get("total")?;
            total -= 1;
            Ok(total)
        } else {
            Ok(0)
        }
    }

    async fn most_players_games(
        &self,
        player: &Uuid,
    ) -> Result<Vec<PerGameStat>, clickhouse_rs::errors::Error> {
        let mut ch_handle = self.clickhouse_pool.get_handle().await?;
        let results = ch_handle
            .query(format!(
                r#"
                SELECT
                    COUNT(DISTINCT player_id) as total,
                    namespace
                FROM
                    (SELECT
                        game_id,
                        namespace
                    FROM player_statistics
                    INNER JOIN games ON player_statistics.game_id = games.game_id
                    WHERE (player_id = '{player_id}')
                        AND (games.date_played < '2023-12-01 00:00:00')
                        AND (games.date_played > '2022-12-31 00:00:00')
                    GROUP BY game_id, namespace) AS games
                INNER JOIN player_statistics ON player_statistics.game_id = games.game_id
                GROUP BY namespace
                ORDER BY total DESC
            "#,
                // safety: player is a uuid, which has a fixed format which is safe to insert directly into the sql
                player_id = player
            ))
            .fetch_all()
            .await?;

        let mut top_games = Vec::with_capacity(results.row_count());

        for row in results.rows() {
            let namespace: String = row.get("namespace")?;
            let mut total: u64 = row.get("total")?;
            total -= 1;
            top_games.push(PerGameStat { namespace, total });
        }

        Ok(top_games)
    }

    pub async fn build_wrapped(
        &self,
        player: &Uuid,
    ) -> Result<PlayerWrappedData, clickhouse_rs::errors::Error> {
        let played_count = self.played_count(player).await?;
        let top_games = self.top_games(player).await?;
        let days_played = self.days_played(player).await?;
        let days_played_games = self.days_played_games(player).await?;
        let most_players = self.most_players(player).await?;
        let most_players_games = self.most_players_games(player).await?;
        Ok(PlayerWrappedData {
            played_count,
            top_games,
            days_played,
            days_played_games,
            most_players,
            most_players_games,
        })
    }
}

#[derive(Deserialize, Serialize)]
pub struct PlayerWrappedData {
    played_count: u64,
    top_games: Vec<PerGameStat>,
    days_played: u64,
    days_played_games: Vec<PerGameStat>,
    most_players: u64,
    most_players_games: Vec<PerGameStat>,
}

#[derive(Deserialize, Serialize)]
pub struct PerGameStat {
    pub namespace: String,
    pub total: u64,
}
