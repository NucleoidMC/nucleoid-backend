use serde::Deserialize;
use uuid::Uuid;
use warp::Filter;
use warp::http::StatusCode;
use xtra::prelude::*;

use crate::controller::*;
use crate::statistics::database::{GetGameStats, GetPlayerStats, GetRecentGames, StatisticsDatabaseError};
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
            move |query: LimitQuery| get_recent_games(controller.clone(), config.clone(), query.limit)
        }).with(&cors);

    let combined = status
        .or(player_game_stats)
        .or(all_player_game_stats)
        .or(all_game_stats)
        .or(get_recent_games);

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
    let statistics = if let Some(statistics) = controller.send(GetStatisticsDatabaseController)
        .await.expect("controller disconnected") {
        statistics
    } else {
        return Ok(send_http_status(StatusCode::NOT_FOUND));
    };

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
    return match res {
        Ok(stats) => {
            Ok(if let Some(stats) = stats {
                Box::new(warp::reply::json(&stats))
            } else {
                send_http_status(StatusCode::NOT_FOUND)
            })
        },
        Err(e) => {
            Ok(handle_server_error(&e))
        }
    }
}

async fn get_game_stats(controller: Address<Controller>, uuid: Uuid) -> ApiResult {
    let statistics = if let Some(statistics) = controller.send(GetStatisticsDatabaseController)
        .await.expect("controller disconnected") {
        statistics
    } else {
        return Ok(send_http_status(StatusCode::NOT_FOUND));
    };

    let res = statistics.send(GetGameStats(uuid)).await.unwrap();
    return match res {
        Ok(stats) => {
            Ok(if let Some(stats) = stats {
                Box::new(warp::reply::json(&stats))
            } else {
                send_http_status(StatusCode::NOT_FOUND)
            })
        },
        Err(e) => {
            Ok(handle_server_error(&e))
        }
    }
}

async fn get_recent_games(controller: Address<Controller>, config: WebServerConfig, limit: u32) -> ApiResult {
    if limit > config.max_query_size {
        return Ok(send_http_status(StatusCode::BAD_REQUEST));
    }

    let statistics = if let Some(statistics) = controller.send(GetStatisticsDatabaseController)
        .await.expect("controller disconnected") {
        statistics
    } else {
        return Ok(send_http_status(StatusCode::NOT_FOUND));
    };

    let res = statistics.send(GetRecentGames(limit)).await.unwrap();
    return match res {
        Ok(games) => {
            Ok(Box::new(warp::reply::json(&games)))
        },
        Err(e) => {
            Ok(handle_server_error(&e))
        }
    }
}

#[derive(Deserialize)]
struct LimitQuery {
    limit: u32,
}

fn handle_server_error(e: &StatisticsDatabaseError) -> Box<dyn warp::Reply> {
    log::warn!("error handling request: {}", e);
    send_http_status(StatusCode::INTERNAL_SERVER_ERROR)
}

fn send_http_status(status: StatusCode) -> Box<dyn warp::Reply> {
    Box::new(warp::reply::with_status(status.canonical_reason().unwrap_or(""), status))
}
