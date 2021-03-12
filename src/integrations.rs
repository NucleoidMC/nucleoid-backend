use std::io;
use std::pin::Pin;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use serde_json;
use tokio::net::{TcpListener, TcpStream};
use xtra::KeepRunning;
use xtra::prelude::*;

use crate::{IntegrationsConfig, TokioGlobal};
use crate::controller::*;
use crate::model::*;

const MAX_FRAME_LENGTH: usize = 4 * 1024 * 1024;
const FRAME_HEADER_SIZE: usize = 4;

pub async fn run(controller: Address<Controller>, config: IntegrationsConfig) {
    let mut listener = TcpListener::bind(&format!("127.0.0.1:{}", config.port)).await
        .expect("failed to open integrations listener");

    loop {
        let (stream, addr) = listener.accept().await
            .expect("failed to accept integrations connection");

        info!("accepting integrations connection from {:?}", addr);

        let controller = controller.clone();
        tokio::spawn(async move {
            match run_client(controller, stream).await {
                Ok(_) => error!("integrations client disconnected"),
                Err(e) => error!("client exited with error: {:?}", e),
            }
        });
    }
}

struct Handshake {
    channel: String,
    game_version: String,
    server_ip: Option<String>,
}

async fn handshake<S: Stream<Item = HandleIncomingMessage> + Unpin>(stream: &mut S) -> Result<Handshake> {
    match stream.next().await {
        Some(HandleIncomingMessage(result)) => {
            match result {
                Ok(IncomingMessage::Handshake { channel, game_version, server_ip }) => Ok(Handshake {
                    channel,
                    game_version,
                    server_ip,
                }),
                Ok(_) => Err(Error::MissingHandshake),
                Err(err) => Err(err),
            }
        }
        None => Err(Error::MissingHandshake),
    }
}

async fn run_client(controller: Address<Controller>, stream: TcpStream) -> Result<()> {
    let (sink, mut stream) = split_framed(stream);
    let handshake = handshake(&mut stream).await?;
    let (channel, game_version, server_ip) = (handshake.channel, handshake.game_version, handshake.server_ip);

    info!("received handshake for: {}", channel);

    let client = IntegrationsClient {
        controller: controller.clone(),
        channel: channel.clone(),
        sink: Box::pin(sink),
    };

    let client = client.create(None).spawn(&mut TokioGlobal);

    controller.do_send_async(RegisterIntegrationsClient { channel, game_version, server_ip, client: client.clone() }).await
        .expect("controller disconnected");

    Ok(client.attach_stream(stream).await)
}

pub struct IntegrationsClient {
    controller: Address<Controller>,
    channel: String,
    sink: Pin<Box<dyn Sink<OutgoingMessage, Error = Error> + Send + Sync>>,
}

#[async_trait]
impl Actor for IntegrationsClient {
    async fn stopping(&mut self, _ctx: &mut Context<Self>) -> KeepRunning {
        let unregister = UnregisterIntegrationsClient { channel: self.channel.clone() };
        let _ = self.controller.do_send_async(unregister).await;
        KeepRunning::StopAll
    }
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", content = "body")]
pub enum IncomingMessage {
    #[serde(rename = "handshake")]
    Handshake {
        channel: String,
        game_version: String,
        server_ip: Option<String>,
    },
    #[serde(rename = "chat")]
    Chat {
        sender: Player,
        content: String,
    },
    #[serde(rename = "status")]
    Status {
        games: Vec<Game>,
        players: Vec<Player>,
    },
    #[serde(rename = "lifecycle_start")]
    LifecycleStart {},
    #[serde(rename = "lifecycle_stop")]
    LifecycleStop {
        crash: bool,
    },
    #[serde(rename = "performance")]
    Performance(ServerPerformance),
    #[serde(rename = "system")]
    SystemMessage {
        content: String,
    }
}

#[derive(Serialize, Debug)]
#[serde(tag = "type", content = "body")]
pub enum OutgoingMessage {
    #[serde(rename = "chat")]
    Chat(ChatMessage),
    #[serde(rename = "command")]
    Command {
        command: String,
        sender: String,
    }
}

impl Message for OutgoingMessage {
    type Result = ();
}

struct HandleIncomingMessage(Result<IncomingMessage>);

impl Message for HandleIncomingMessage {
    type Result = ();
}

#[async_trait]
impl Handler<HandleIncomingMessage> for IntegrationsClient {
    async fn handle(&mut self, message: HandleIncomingMessage, ctx: &mut Context<Self>) {
        match message.0 {
            Ok(message) => {
                use IncomingMessage::*;
                let result = match message {
                    Chat { sender, content } => {
                        let incoming_chat = IncomingChat { channel: self.channel.clone(), sender, content };
                        self.controller.do_send_async(incoming_chat).await
                    }
                    Status { games, players } => {
                        let status_update = StatusUpdate { channel: self.channel.clone(), games, players };
                        self.controller.do_send_async(status_update).await
                    }
                    LifecycleStart {} => {
                        let lifecycle = ServerLifecycleStart { channel: self.channel.clone() };
                        self.controller.do_send_async(lifecycle).await
                    }
                    LifecycleStop { crash } => {
                        let lifecycle = ServerLifecycleStop { channel: self.channel.clone(), crash };
                        self.controller.do_send_async(lifecycle).await
                    }
                    Performance(performance) => {
                        let performance_update = PerformanceUpdate { channel: self.channel.clone(), performance };
                        self.controller.do_send_async(performance_update).await
                    }
                    SystemMessage { content  } => {
                        let system_message = ServerSystemMessage { channel: self.channel.clone(), content };
                        self.controller.do_send_async(system_message).await
                    }
                    _ => {
                        warn!("received unexpected message from integrations client: {:?}", message);
                        Ok(())
                    }
                };

                if result.is_err() {
                    ctx.stop();
                }
            }
            Err(Error::Json(err)) => {
                warn!("malformed message from client: {:?}", err);
            }
            Err(err) => {
                error!("integrations client closing with error: {:?}", err);
                ctx.stop();
            }
        }
    }
}

#[async_trait]
impl Handler<OutgoingMessage> for IntegrationsClient {
    async fn handle(&mut self, message: OutgoingMessage, _ctx: &mut Context<Self>) {
        // TODO: how should we handle errors here?
        let _ = self.sink.send(message).await;
    }
}

fn split_framed(stream: TcpStream) -> (impl Sink<OutgoingMessage, Error = Error> + Send, impl Stream<Item = HandleIncomingMessage>) {
    let (sink, stream) = tokio_util::codec::LengthDelimitedCodec::builder()
        .big_endian()
        .max_frame_length(MAX_FRAME_LENGTH)
        .length_field_length(FRAME_HEADER_SIZE)
        .num_skip(FRAME_HEADER_SIZE)
        .length_field_offset(0)
        .length_adjustment(0)
        .new_framed(stream)
        .split();

    let sink = sink.with(|message: OutgoingMessage| async move {
        let mut bytes = Vec::with_capacity(64);
        serde_json::to_writer(&mut bytes, &message)?;
        Ok(Bytes::from(bytes))
    });

    let stream = stream.map(|result| {
        HandleIncomingMessage(match result {
            Ok(bytes) => serde_json::from_slice(bytes.as_ref()).map_err(Error::Json),
            Err(err) => Err(err.into()),
        })
    });

    (sink, stream)
}

type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("io error")]
    Io(#[from] io::Error),
    #[error("invalid json")]
    Json(#[from] serde_json::Error),
    #[error("missing handshake")]
    MissingHandshake,
}
