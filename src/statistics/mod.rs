use xtra::{Address, Actor};
use crate::{Controller, StatisticsConfig, TokioGlobal, RegisterStatisticsDatabaseController};
use crate::statistics::database::StatisticDatabaseController;

pub mod model;
pub mod database;

pub async fn run(controller: Address<Controller>, config: StatisticsConfig) {
    let statistics_database = StatisticDatabaseController::connect(&controller, &config).await
        .expect("failed to connect to statistics database")
        .create(None)
        .spawn(&mut TokioGlobal);

    controller.do_send_async(RegisterStatisticsDatabaseController { controller: statistics_database })
        .await.expect("controller disconnected");
}
