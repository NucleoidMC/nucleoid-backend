use warp::Filter;
use warp::http::StatusCode;
use xtra::prelude::*;

use crate::WebServerConfig;
use crate::controller::*;

pub async fn run(controller: Address<Controller>, config: WebServerConfig) {
    let cors = warp::cors()
        .allow_any_origin();

    let status = warp::path("status")
        .and(warp::path::param())
        .and_then({
            let controller = controller.clone();
            move |channel| get_status(controller.clone(), channel)
        });

    warp::serve(status.with(cors))
        .run(([127, 0, 0, 1], config.port))
        .await;
}

async fn get_status(controller: Address<Controller>, channel: String) -> Result<Box<dyn warp::Reply>, warp::Rejection> {
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
