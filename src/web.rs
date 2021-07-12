use uuid::Uuid;
use warp::Filter;
use warp::http::StatusCode;
use xtra::prelude::*;

use crate::controller::*;
use crate::statistics::database::GetPlayerStats;
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

    let player_game_stats = warp::path("player")
        .and(warp::path::param::<Uuid>())
        .and(warp::path("stats"))
        .and(warp::path::param::<String>())
        .and_then({
            let controller = controller.clone();
            move |uuid, namespace| get_player_stats(controller.clone(), uuid, Some(namespace))
        }).with(&cors);

    let all_player_game_stats = warp::path("player")
        .and(warp::path::param::<Uuid>())
        .and(warp::path("stats"))
        .and_then({
            let controller = controller.clone();
            move |uuid| get_player_stats(controller.clone(), uuid, None)
        }).with(&cors);

    let combined = status
        .or(player_game_stats)
        .or(all_player_game_stats);

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

    let res = statistics.send(GetPlayerStats {
        uuid,
        namespace
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

fn handle_server_error(e: &anyhow::Error) -> Box<dyn warp::Reply> {
    log::warn!("error handling request: {}", e);
    send_http_status(StatusCode::INTERNAL_SERVER_ERROR)
}

fn send_http_status(status: StatusCode) -> Box<dyn warp::Reply> {
    Box::new(warp::reply::with_status(status.canonical_reason().unwrap_or(""), status))
}
