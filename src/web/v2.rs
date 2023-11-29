use warp::{Filter, Reply};
use xtra::prelude::*;

use crate::{Controller, statistics::database::GetLeaderboardV2};

use super::{get_statistics_controller, handle_option_result, ApiResult};

pub fn build_v2(controller: Address<Controller>) -> warp::filters::BoxedFilter<(impl Reply,)> {
    let cors = warp::cors().allow_any_origin();

    let leaderboards = warp::path("leaderboard")
        .and(warp::path::param::<String>())
        .and_then({
            let controller = controller.clone();
            move |id| get_leaderboard(controller.clone(), id)
        })
        .with(&cors)
        .boxed();

    leaderboards
}

async fn get_leaderboard(controller: Address<Controller>, id: String) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(GetLeaderboardV2(id))
        .await
        .expect("controller disconnected");
    handle_option_result(res)
}
