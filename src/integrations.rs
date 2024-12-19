use std::io;
use std::pin::Pin;

use bytes::Bytes;
use futures::{Sink, SinkExt, Stream, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};

use tokio::net::{TcpListener, TcpStream};
use xtra::prelude::*;

use crate::controller::*;
use crate::model::*;
use crate::statistics::database::UploadStatsBundle;
use crate::statistics::model::GameStatsBundle;
use crate::IntegrationsConfig;
use uuid::Uuid;

const MAX_FRAME_LENGTH: usize = 4 * 1024 * 1024;
const FRAME_HEADER_SIZE: usize = 4;

pub async fn run(controller: Address<Controller>, config: IntegrationsConfig) {
    let listener = TcpListener::bind(&format!("0.0.0.0:{}", config.port))
        .await
        .expect("failed to open integrations listener");

    loop {
        let (stream, addr) = listener
            .accept()
            .await
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
    server_type: ServerType,
}

async fn handshake<S: Stream<Item = HandleIncomingMessage> + Unpin>(
    stream: &mut S,
) -> Result<Handshake> {
    match stream.next().await {
        Some(HandleIncomingMessage(result)) => match result {
            Ok(IncomingMessage::Handshake {
                channel,
                game_version,
                server_ip,
                server_type,
            }) => Ok(Handshake {
                channel,
                game_version,
                server_ip,
                server_type: server_type.unwrap_or(ServerType::Minecraft),
            }),
            Ok(_) => Err(Error::MissingHandshake),
            Err(err) => Err(err),
        },
        None => Err(Error::MissingHandshake),
    }
}

async fn run_client(controller: Address<Controller>, stream: TcpStream) -> Result<()> {
    let (sink, mut stream) = split_framed(stream);
    let handshake = handshake(&mut stream).await?;
    let (channel, game_version, server_ip, server_type) = (
        handshake.channel,
        handshake.game_version,
        handshake.server_ip,
        handshake.server_type,
    );

    info!(
        "received handshake for: {} (type: {:?})",
        channel, server_type
    );

    let client = IntegrationsClient {
        controller: controller.clone(),
        channel: channel.clone(),
        sink: Box::pin(sink),
        server_type,
    };

    let client = xtra::spawn_tokio(client, Mailbox::unbounded());

    controller
        .send(RegisterIntegrationsClient {
            channel,
            game_version,
            server_ip,
            client: client.clone(),
        })
        .await
        .expect("controller disconnected");

    if let Err(e) = stream.map(|m| Ok(m)).forward(client.into_sink()).await {
        log::error!("error in integrations client: {e}");
    }

    Ok(())
}

pub struct IntegrationsClient {
    controller: Address<Controller>,
    channel: String,
    sink: Pin<Box<dyn Sink<OutgoingMessage, Error = Error> + Send + Sync>>,
    server_type: ServerType,
}

impl Actor for IntegrationsClient {
    type Stop = ();

    async fn stopped(self) -> Self::Stop {
        let unregister = UnregisterIntegrationsClient {
            channel: self.channel.clone(),
        };
        let _ = self.controller.send(unregister).await;
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
        server_type: Option<ServerType>,
    },
    #[serde(rename = "chat")]
    Chat { sender: Player, content: String },
    #[serde(rename = "status")]
    Status {
        #[serde(default)]
        players: Option<Vec<Player>>,
        #[serde(default)]
        games: Option<Vec<Game>>,
    },
    #[serde(rename = "lifecycle_start")]
    LifecycleStart {},
    #[serde(rename = "lifecycle_stop")]
    LifecycleStop { crash: bool },
    #[serde(rename = "performance")]
    Performance(ServerPerformance),
    #[serde(rename = "system")]
    SystemMessage { content: String },
    #[serde(rename = "upload_statistics")]
    UploadStatistics {
        bundle: GameStatsBundle,
        game_id: Uuid,
    },
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
        roles: Vec<String>,
        silent: bool,
    },
    #[serde(rename = "send_to_server")]
    SendToServer {
        // The UUID
        player: String,
        target_server: String,
    },
    #[serde(rename = "send_server_to_server")]
    SendServerToServer {
        from_server: String,
        to_server: String,
    },
}

struct HandleIncomingMessage(Result<IncomingMessage>);


impl Handler<HandleIncomingMessage> for IntegrationsClient {
    type Return = ();

    async fn handle(&mut self, message: HandleIncomingMessage, ctx: &mut Context<Self>) {
        match message.0 {
            Ok(message) => {
                use IncomingMessage::*;
                let result = match message {
                    Chat { sender, content } => {
                        let incoming_chat = IncomingChat {
                            channel: self.channel.clone(),
                            sender,
                            content,
                        };
                        self.controller.send(incoming_chat).await
                    }
                    Status { games, players } => {
                        let status_update = StatusUpdate {
                            channel: self.channel.clone(),
                            games,
                            players,
                        };
                        self.controller.send(status_update).await
                    }
                    LifecycleStart {} => {
                        let lifecycle = ServerLifecycleStart {
                            channel: self.channel.clone(),
                            server_type: self.server_type.clone(),
                        };
                        self.controller.send(lifecycle).await
                    }
                    LifecycleStop { crash } => {
                        let lifecycle = ServerLifecycleStop {
                            channel: self.channel.clone(),
                            crash,
                            server_type: self.server_type.clone(),
                        };
                        self.controller.send(lifecycle).await
                    }
                    Performance(performance) => {
                        let performance_update = PerformanceUpdate {
                            channel: self.channel.clone(),
                            performance,
                        };
                        self.controller.send(performance_update).await
                    }
                    SystemMessage { content } => {
                        let system_message = ServerSystemMessage {
                            channel: self.channel.clone(),
                            content,
                        };
                        self.controller.send(system_message).await
                    }
                    UploadStatistics { bundle, game_id } => {
                        if let Some(global) = &bundle.stats.global {
                            log::debug!("server '{}' uploaded {} player statistics and {} global statistics in statistics bundle for {}",
                                self.channel, bundle.stats.players.len(), global.len(), bundle.namespace);
                        } else {
                            log::debug!("server '{}' uploaded {} player statistics in statistics bundle for {}",
                                self.channel, bundle.stats.players.len(), bundle.namespace);
                        }
                        let upload_bundle_message = UploadStatsBundle {
                            game_id,
                            bundle,
                            server: self.channel.clone(),
                        };
                        self.controller.send(upload_bundle_message).await
                    }
                    _ => {
                        warn!(
                            "received unexpected message from integrations client: {:?}",
                            message
                        );
                        Ok(())
                    }
                };

                if result.is_err() {
                    ctx.stop_self();
                }
            }
            Err(Error::Json(err)) => {
                warn!("malformed message from client: {:?}", err);
            }
            Err(err) => {
                error!("integrations client closing with error: {:?}", err);
                ctx.stop_self();
            }
        }
    }
}

impl Handler<OutgoingMessage> for IntegrationsClient {
    type Return = ();

    async fn handle(&mut self, message: OutgoingMessage, _ctx: &mut Context<Self>) {
        // TODO: how should we handle errors here?
        let _ = self.sink.send(message).await;
    }
}

fn split_framed(
    stream: TcpStream,
) -> (
    impl Sink<OutgoingMessage, Error = Error> + Send,
    impl Stream<Item = HandleIncomingMessage>,
) {
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
