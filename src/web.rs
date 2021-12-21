use serde::{Deserialize, Serialize};
use uuid::Uuid;
use warp::Filter;
use warp::http::StatusCode;
use xtra::prelude::*;

use crate::controller::*;
use crate::statistics::database::{GetGameStats, GetPlayerStats, GetRecentGames, GetStatisticsStats, StatisticDatabaseController, StatisticsDatabaseError, StatisticsDatabaseResult, UpdateLeaderboards};
use crate::WebServerConfig;

pub async fn run(controller: Address<Controller>, config: WebServerConfig) {
    let cors = warp::cors()
        .allow_any_origin();

    let status = warp::path("status")
        .and(warp::path::param())
        .and_then({
            let controller = controller.clone();
            move |channel| get_status(controller.clone(), channel)
        }).with(&cors);

    let player_game_stats = warp::path("stats")
        .and(warp::path("player"))
        .and(warp::path::param::<Uuid>())
        .and(warp::path::param::<String>())
        .and_then({
            let controller = controller.clone();
            move |uuid, namespace| get_player_stats(controller.clone(), uuid, Some(namespace))
        }).with(&cors);

    let all_player_game_stats = warp::path("stats")
        .and(warp::path("player"))
        .and(warp::path::param::<Uuid>())
        .and_then({
            let controller = controller.clone();
            move |uuid| get_player_stats(controller.clone(), uuid, None)
        }).with(&cors);

    let all_game_stats = warp::path("stats")
        .and(warp::path("game"))
        .and(warp::path::param::<Uuid>())
        .and_then({
            let controller = controller.clone();
            move |uuid| get_game_stats(controller.clone(), uuid)
        }).with(&cors);

    let get_recent_games = warp::path("games")
        .and(warp::path("recent"))
        .and(warp::query::query())
        .and_then({
            let controller = controller.clone();
            let config = config.clone();
            move |query: RecentGamesQuery| get_recent_games(controller.clone(), config.clone(), query)
        }).with(&cors);

    // let leaderboard = warp::path("leaderboard")
    //     .and(warp::path::param::<String>())
    //     .and_then({
    //         let controller = controller.clone();
    //         move |id| get_leaderboard(controller.clone(), id)
    //     }).with(&cors);

    let update_leaderboards = warp::path("leaderboards")
        .and(warp::path("update"))
        .and_then({
            let controller = controller.clone();
            move || update_leaderboards(controller.clone())
        });

    let get_statistics_stats = warp::path("stats")
        .and(warp::path("stats"))
        .and_then({
            let controller = controller.clone();
            move || get_statistics_stats(controller.clone())
        }).with(&cors);

    let combined = status
        .or(player_game_stats)
        .or(all_player_game_stats)
        .or(all_game_stats)
        .or(get_recent_games)
        .or(get_statistics_stats)
        .or(update_leaderboards);
        // .or(leaderboard);

    warp::serve(combined)
        .run(([127, 0, 0, 1], config.port))
        .await;
}

async fn get_status(controller: Address<Controller>, channel: String) -> ApiResult {
    match controller.send(GetStatus(channel)).await {
        Ok(status) => {
            Ok(match status {
                Some(status) => Box::new(warp::reply::json(&status)),
                None => Box::new(warp::reply::with_status("Not found", StatusCode::NOT_FOUND)),
            })
        },
        Err(err) => Ok(Box::new(warp::reply::with_status(format!("{:?}", err), StatusCode::INTERNAL_SERVER_ERROR))),
    }
}

type ApiResult = Result<Box<dyn warp::Reply>, warp::Rejection>;

async fn get_player_stats(controller: Address<Controller>, uuid: Uuid, namespace: Option<String>) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;

    if let Some(namespace) = &namespace {
        for c in namespace.chars() {
            if !((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_') {
                return Ok(send_http_status(StatusCode::BAD_REQUEST));
            }
        }
    }

    let res = statistics.send(GetPlayerStats {
        uuid,
        namespace,
    }).await.unwrap();
    handle_statistics_option_result(res)
}

async fn get_game_stats(controller: Address<Controller>, uuid: Uuid) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics.send(GetGameStats(uuid)).await.unwrap();
    handle_statistics_option_result(res)
}

async fn get_recent_games(controller: Address<Controller>, config: WebServerConfig, query: RecentGamesQuery) -> ApiResult {
    if query.limit > config.max_query_size {
        return Ok(send_http_status(StatusCode::BAD_REQUEST));
    }

    let statistics = get_statistics_controller(controller).await?;

    let res = statistics.send(GetRecentGames {
        limit: query.limit,
        player_id: query.player,
    }).await.unwrap();
    handle_statistics_result(res)
}

async fn get_statistics_stats(controller: Address<Controller>) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics.send(GetStatisticsStats).await.expect("controller disconnected");
    handle_statistics_result(res)
}

// async fn get_leaderboard(controller: Address<Controller>, id: String) -> ApiResult {
//     let statistics = get_statistics_controller(controller).await?;
//     let res = statistics.send(GetLeaderboard(id)).await.expect("controller disconnected");
//     handle_statistics_option_result(res)
// }

async fn update_leaderboards(controller: Address<Controller>) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics.send(UpdateLeaderboards).await.expect("controller disconnected");
    handle_statistics_result(res.map(|_| "ok"))
}

#[derive(Deserialize)]
struct RecentGamesQuery {
    limit: u32,
    player: Option<Uuid>,
}

async fn get_statistics_controller(controller: Address<Controller>) -> Result<Address<StatisticDatabaseController>, warp::Rejection> {
    if let Some(statistics) = controller.send(GetStatisticsDatabaseController).await.expect("controller disconnected") {
        Ok(statistics)
    } else {
        Err(warp::reject::not_found())
    }
}

fn handle_statistics_result<T>(result: StatisticsDatabaseResult<T>) -> ApiResult
    where T: Serialize {
    match result {
        Ok(t) => Ok(Box::new(warp::reply::json(&t))),
        Err(e) => Ok(handle_server_error(&e)),
    }
}

fn handle_statistics_option_result<T>(result: StatisticsDatabaseResult<Option<T>>) -> ApiResult
    where T: Serialize {
    match result {
        Ok(Some(t)) => Ok(Box::new(warp::reply::json(&t))),
        Ok(None) => Err(warp::reject::not_found()),
        Err(e) => Ok(handle_server_error(&e)),
    }
}

fn handle_server_error(e: &StatisticsDatabaseError) -> Box<dyn warp::Reply> {
    log::warn!("error handling request: {}", e);
    send_http_status(StatusCode::INTERNAL_SERVER_ERROR)
}

fn send_http_status(status: StatusCode) -> Box<dyn warp::Reply> {
    Box::new(warp::reply::with_status(status.canonical_reason().unwrap_or(""), status))
}
