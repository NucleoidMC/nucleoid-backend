use serde::{Deserialize, Serialize};
use std::error::Error;
use std::num::NonZeroUsize;
use uuid::Uuid;
use warp::http::StatusCode;
use warp::Filter;
use xtra::prelude::*;

use crate::controller::*;
use crate::mojang_api::{GetPlayerUsername, MojangApiClient};
use crate::statistics::database::*;
use crate::statistics::model::DataQueryType;
use crate::WebServerConfig;

pub async fn run(controller: Address<Controller>, config: WebServerConfig) {
    let cors = warp::cors().allow_any_origin();

    let mojang_client = MojangApiClient::start(NonZeroUsize::new(512).unwrap())
        .expect("failed to create Mojang API client");

    let status = warp::path("status")
        .and(warp::path::param())
        .and_then({
            let controller = controller.clone();
            move |channel| get_status(controller.clone(), channel)
        })
        .with(&cors);

    let player_game_stats = warp::path("stats")
        .and(warp::path("player"))
        .and(warp::path::param::<Uuid>())
        .and(warp::path::param::<String>())
        .and_then({
            let controller = controller.clone();
            move |uuid, namespace| get_player_stats(controller.clone(), uuid, Some(namespace))
        })
        .with(&cors);

    let all_player_game_stats = warp::path("stats")
        .and(warp::path("player"))
        .and(warp::path::param::<Uuid>())
        .and_then({
            let controller = controller.clone();
            move |uuid| get_player_stats(controller.clone(), uuid, None)
        })
        .with(&cors);

    let all_game_stats = warp::path("stats")
        .and(warp::path("game"))
        .and(warp::path::param::<Uuid>())
        .and_then({
            let controller = controller.clone();
            move |uuid| get_game_stats(controller.clone(), uuid)
        })
        .with(&cors);

    let get_recent_games = warp::path("games")
        .and(warp::path("recent"))
        .and(warp::query::query())
        .and_then({
            let controller = controller.clone();
            let config = config.clone();
            move |query: RecentGamesQuery| {
                get_recent_games(controller.clone(), config.clone(), query)
            }
        })
        .with(&cors);

    let get_leaderboard = warp::path("leaderboard")
        .and(warp::path::param::<String>())
        .and_then({
            let controller = controller.clone();
            move |id| get_leaderboard(controller.clone(), id)
        })
        .with(&cors);

    let list_leaderboards = warp::path("leaderboards")
        .and_then({
            let controller = controller.clone();
            move || list_leaderboards(controller.clone())
        })
        .with(&cors);

    let get_player_rankings = warp::path("player")
        .and(warp::path::param::<Uuid>())
        .and(warp::path("rankings"))
        .and_then({
            let controller = controller.clone();
            move |id| get_player_rankings(controller.clone(), id)
        })
        .with(&cors);

    let get_statistics_stats = warp::path("stats")
        .and(warp::path("stats"))
        .and_then({
            let controller = controller.clone();
            move || get_statistics_stats(controller.clone())
        })
        .with(&cors);

    let data_query = warp::path("stats")
        .and(warp::path("data"))
        .and(warp::path("query"))
        .and(warp::query())
        .and(warp::get())
        .and_then({
            let controller = controller.clone();
            move |query: DataQueryQuery| data_query(controller.clone(), query)
        })
        .with(&cors);

    let get_player_username = warp::path("player")
        .and(warp::path::param::<Uuid>())
        .and(warp::path("username"))
        .and_then({
            let mojang_client = mojang_client.clone();
            move |id| get_player_username(mojang_client.clone(), id)
        })
        .with(&cors);

    let nucleoid_wrapped = warp::path("player")
        .and(warp::path::param::<Uuid>())
        .and(warp::path("wrapped"))
        .and_then({
            let controller = controller.clone();
            move |id| nucleoid_wrapped(controller.clone(), id)
        })
        .with(&cors);

    let combined = status
        .or(player_game_stats)
        .or(all_player_game_stats)
        .or(all_game_stats)
        .or(get_recent_games)
        .or(get_statistics_stats)
        .or(get_leaderboard)
        .or(list_leaderboards)
        .or(get_player_rankings)
        .or(data_query)
        .or(get_player_username)
        .or(nucleoid_wrapped);

    warp::serve(combined)
        .run(([127, 0, 0, 1], config.port))
        .await;
}

async fn get_status(controller: Address<Controller>, channel: String) -> ApiResult {
    match controller.send(GetStatus(channel)).await {
        Ok(status) => Ok(match status {
            Some(status) => Box::new(warp::reply::json(&status)),
            None => Box::new(warp::reply::with_status("Not found", StatusCode::NOT_FOUND)),
        }),
        Err(err) => Ok(Box::new(warp::reply::with_status(
            format!("{:?}", err),
            StatusCode::INTERNAL_SERVER_ERROR,
        ))),
    }
}

type ApiResult = Result<Box<dyn warp::Reply>, warp::Rejection>;

async fn get_player_stats(
    controller: Address<Controller>,
    uuid: Uuid,
    namespace: Option<String>,
) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;

    if let Some(namespace) = &namespace {
        for c in namespace.chars() {
            if !(c.is_ascii_lowercase() || c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            {
                return Ok(send_http_status(StatusCode::BAD_REQUEST));
            }
        }
    }

    let res = statistics
        .send(GetPlayerStats { uuid, namespace })
        .await
        .unwrap();
    handle_option_result(res)
}

async fn get_game_stats(controller: Address<Controller>, uuid: Uuid) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics.send(GetGameStats(uuid)).await.unwrap();
    handle_option_result(res)
}

async fn get_recent_games(
    controller: Address<Controller>,
    config: WebServerConfig,
    query: RecentGamesQuery,
) -> ApiResult {
    if query.limit > config.max_query_size {
        return Ok(send_http_status(StatusCode::BAD_REQUEST));
    }

    let statistics = get_statistics_controller(controller).await?;

    let res = statistics
        .send(GetRecentGames {
            limit: query.limit,
            player_id: query.player,
        })
        .await
        .unwrap();
    handle_result(res)
}

async fn get_statistics_stats(controller: Address<Controller>) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(GetStatisticsStats)
        .await
        .expect("controller disconnected");
    handle_result(res)
}

async fn get_leaderboard(controller: Address<Controller>, id: String) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(GetLeaderboard(id))
        .await
        .expect("controller disconnected");
    handle_option_result(res)
}

async fn list_leaderboards(controller: Address<Controller>) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(GetAllLeaderboards)
        .await
        .expect("controller disconnected");
    Ok(Box::new(warp::reply::json(&res)))
}

async fn get_player_rankings(controller: Address<Controller>, player: Uuid) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(GetPlayerRankings(player))
        .await
        .expect("controller disconnected");
    handle_option_result(res)
}

async fn data_query(controller: Address<Controller>, query: DataQueryQuery) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(DataQuery(query.query))
        .await
        .expect("controller disconnected");
    handle_result(res.map(|r| serde_json::json!({ "data": r })))
}

async fn get_player_username(mojang_client: Address<MojangApiClient>, id: Uuid) -> ApiResult {
    let profile = mojang_client
        .send(GetPlayerUsername(id))
        .await
        .expect("Mojang client disconnected");
    handle_option_result(profile)
}

async fn nucleoid_wrapped(controller: Address<Controller>, player_id: Uuid) -> ApiResult {
    let statistics = get_statistics_controller(controller).await?;
    let res = statistics
        .send(WrappedData(player_id))
        .await
        .expect("controller disconnected");
    handle_result(res)
}

#[derive(Deserialize)]
struct RecentGamesQuery {
    limit: u32,
    player: Option<Uuid>,
}

#[derive(Deserialize)]
struct DataQueryQuery {
    query: DataQueryType,
}

async fn get_statistics_controller(
    controller: Address<Controller>,
) -> Result<Address<StatisticDatabaseController>, warp::Rejection> {
    if let Some(statistics) = controller
        .send(GetStatisticsDatabaseController)
        .await
        .expect("controller disconnected")
    {
        Ok(statistics)
    } else {
        Err(warp::reject::not_found())
    }
}

fn handle_result<T, E>(result: Result<T, E>) -> ApiResult
where
    T: Serialize,
    E: Error,
{
    match result {
        Ok(t) => Ok(Box::new(warp::reply::json(&t))),
        Err(e) => Ok(handle_server_error(&e)),
    }
}

fn handle_option_result<T, E>(result: Result<Option<T>, E>) -> ApiResult
where
    T: Serialize,
    E: Error,
{
    match result {
        Ok(Some(t)) => Ok(Box::new(warp::reply::json(&t))),
        Ok(None) => Err(warp::reject::not_found()),
        Err(e) => Ok(handle_server_error(&e)),
    }
}

fn handle_server_error<E>(e: &E) -> Box<dyn warp::Reply>
where
    E: Error,
{
    log::warn!("error handling request: {}", e);
    send_http_status(StatusCode::INTERNAL_SERVER_ERROR)
}

fn send_http_status(status: StatusCode) -> Box<dyn warp::Reply> {
    Box::new(warp::reply::with_status(
        status.canonical_reason().unwrap_or(""),
        status,
    ))
}
