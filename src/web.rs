use async_trait::async_trait;
use warp::Filter;
use warp::http::StatusCode;
use xtra::prelude::*;

use crate::Config;
use crate::controller::*;

pub async fn run(controller: Address<Controller>, config: Config) {
    let status = warp::path("status")
        .and(warp::path::param())
        .and_then({
            let controller = controller.clone();
            move |channel| get_status(controller.clone(), channel)
        });

    warp::serve(status)
        .run(([127, 0, 0, 1], config.web_server_port))
        .await;
}

async fn get_status(controller: Address<Controller>, channel: String) -> Result<Box<dyn warp::Reply>, warp::Rejection> {
    match controller.send(GetStatus(channel)).await {
        Ok(status) => Ok(Box::new(warp::reply::json(&status))),
        Err(err) => Ok(Box::new(warp::reply::with_status(format!("{:?}", err), StatusCode::INTERNAL_SERVER_ERROR))),
    }
}
