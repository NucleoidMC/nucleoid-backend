use std::sync::Arc;

use async_trait::async_trait;
use warp::Filter;
use xtra::prelude::*;

use crate::{Config, Controller};

pub struct WebController {
    controller: Option<Address<Controller>>,
}

impl Actor for WebController {
}

pub struct Start(pub Address<Controller>);

impl Message for Start {
    type Result = ();
}

#[async_trait]
impl Handler<Start> for WebController {
    async fn handle(&mut self, start: Start, ctx: &mut Context<Self>) {
        let controller = start.0;
        self.controller = Some(controller);
    }
}

pub async fn run(controller: Arc<Controller>) {
    let hello = warp::path!("hello" / String)
        .map(|name| format!("Hello, {}!", name));

    warp::serve(hello)
        .run(([127, 0, 0, 1], controller.config.web_server_port))
        .await;
}
