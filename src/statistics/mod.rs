use std::fs::File;

use nucleoid_leaderboards::model::LeaderboardDefinition;
use walkdir::WalkDir;
use xtra::{Address, Mailbox};

use crate::statistics::database::StatisticDatabaseController;
use crate::{Controller, RegisterStatisticsDatabaseController, StatisticsConfig};

pub mod database;
pub mod leaderboards;
pub mod model;
mod wrapped;

pub async fn run(
    controller: Address<Controller>,
    config: StatisticsConfig,
    postgres_pool: deadpool_postgres::Pool,
) {
    let statistics_database = StatisticDatabaseController::connect(
        &controller,
        postgres_pool,
        &config,
        load_leaderboards(&config),
    )
    .await
    .expect("failed to connect to statistics database");

    let statistics_database = xtra::spawn_tokio(statistics_database, Mailbox::unbounded());

    controller
        .send(RegisterStatisticsDatabaseController {
            controller: statistics_database,
        })
        .await
        .expect("controller disconnected");
}

fn load_leaderboards(config: &StatisticsConfig) -> Vec<LeaderboardDefinition> {
    let mut leaderboards = Vec::new();

    if let Some(leaderboards_dir) = &config.leaderboards_dir {
        for entry in WalkDir::new(leaderboards_dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let filename = entry.file_name().to_string_lossy();
            if filename.ends_with(".json") {
                let file = match File::open(entry.path()) {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::error!("Failed to open {:?}: {}", entry.path(), e);
                        continue;
                    }
                };
                match serde_json::from_reader::<_, LeaderboardDefinition>(&file) {
                    Ok(definition) => leaderboards.push(definition),
                    Err(e) => tracing::error!("Failed to parse {:?}: {}", entry.path(), e),
                }
            }
        }
    }

    leaderboards
}
