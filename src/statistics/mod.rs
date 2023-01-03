use std::fs::File;

use nucleoid_leaderboards::model::LeaderboardDefinition;
use walkdir::WalkDir;
use xtra::{Actor, Address};

use crate::statistics::database::StatisticDatabaseController;
use crate::{Controller, RegisterStatisticsDatabaseController, StatisticsConfig, TokioGlobal};

pub mod database;
pub mod leaderboards;
pub mod model;

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
    .expect("failed to connect to statistics database")
    .create(None)
    .spawn(&mut TokioGlobal);

    controller
        .do_send_async(RegisterStatisticsDatabaseController {
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
                        log::error!("Failed to open {:?}: {}", entry.path(), e);
                        continue;
                    }
                };
                match serde_json::from_reader::<_, LeaderboardDefinition>(&file) {
                    Ok(definition) => leaderboards.push(definition),
                    Err(e) => log::error!("Failed to parse {:?}: {}", entry.path(), e),
                }
            }
        }
    }

    leaderboards
}
