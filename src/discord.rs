use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use log::{error, info, warn};
use serde_json::json;
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Embed, Message as SerenityMessage, Reaction};
use serenity::prelude::*;
use serenity::CacheAndHttp;
use xtra::prelude::*;
use xtra::Context as XtraContext;
use xtra::KeepRunning;
use xtra::Message as XtraMessage;

use crate::controller::*;
use crate::model::*;
use crate::{DiscordConfig, Persistent, TokioGlobal};

mod lfp;
mod pings;
mod relay;

pub struct DiscordClient {
    controller: Address<Controller>,
    config: DiscordConfig,
    cache_and_http: Option<Arc<CacheAndHttp>>,
    data: Option<Arc<RwLock<TypeMap>>>,
}

#[async_trait]
impl Actor for DiscordClient {
    async fn stopping(&mut self, _ctx: &mut XtraContext<Self>) -> KeepRunning {
        let _ = self.controller.do_send_async(UnregisterDiscordClient).await;
        KeepRunning::StopAll
    }
}

pub async fn run(controller: Address<Controller>, config: DiscordConfig) {
    let relay_store = Persistent::open("relay.json").await;
    let ping_store = Persistent::open("pings.json").await;
    let lfp_store = Persistent::open("lfp.json").await;

    let actor = DiscordClient {
        controller: controller.clone(),
        config: config.clone(),
        cache_and_http: None,
        data: None,
    };
    let address = actor.create(None).spawn(&mut TokioGlobal);

    let handler = DiscordHandler {
        pings: pings::Handler {
            discord: address.clone(),
        },
        relay: relay::Handler {
            controller: controller.clone(),
            discord: address.clone(),
        },
        lfp: lfp::Handler {
            discord: address.clone(),
            config: config.clone(),
        },
    };

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGE_REACTIONS;

    let mut client = Client::builder(config.token, intents)
        .event_handler(handler)
        .await
        .expect("failed to create client");

    {
        let mut data = client.data.write().await;
        data.insert::<relay::StoreKey>(relay_store);
        data.insert::<pings::StoreKey>(ping_store);
        data.insert::<lfp::StoreKey>(lfp_store);
    }

    address
        .do_send_async(Init {
            cache_and_http: client.cache_and_http.clone(),
            data: client.data.clone(),
        })
        .await
        .expect("client disconnected");

    controller
        .do_send_async(RegisterDiscordClient { client: address })
        .await
        .expect("controller disconnected");

    client.start().await.expect("failed to run client");
}

struct Init {
    cache_and_http: Arc<CacheAndHttp>,
    data: Arc<RwLock<TypeMap>>,
}

impl XtraMessage for Init {
    type Result = ();
}

pub struct SendChat {
    pub channel: String,
    pub sender: Player,
    pub content: String,
}

impl XtraMessage for SendChat {
    type Result = ();
}

pub struct SendSystem {
    pub channel: String,
    pub content: String,
}

impl XtraMessage for SendSystem {
    type Result = ();
}

pub struct SendPing {
    pub ping: String,
    pub sender_name: String,
    pub sender_icon: Option<String>,
    pub content: String,
}

impl XtraMessage for SendPing {
    type Result = ();
}

pub struct UpdateRelayStatus {
    pub channel: String,
    pub game_version: String,
    pub server_ip: Option<String>,
    pub player_count: usize,
}

impl XtraMessage for UpdateRelayStatus {
    type Result = ();
}

pub struct ReportError {
    pub title: String,
    pub description: String,
    pub fields: Option<HashMap<String, String>>,
}

impl XtraMessage for ReportError {
    type Result = ();
}

#[async_trait]
impl Handler<Init> for DiscordClient {
    async fn handle(&mut self, message: Init, _ctx: &mut XtraContext<Self>) {
        self.cache_and_http = Some(message.cache_and_http);
        self.data = Some(message.data);
    }
}

#[async_trait]
impl Handler<SendChat> for DiscordClient {
    async fn handle(&mut self, send_chat: SendChat, _ctx: &mut XtraContext<Self>) {
        relay::send_chat(self, send_chat).await
    }
}

#[async_trait]
impl Handler<SendSystem> for DiscordClient {
    async fn handle(&mut self, send_system: SendSystem, _ctx: &mut XtraContext<Self>) {
        relay::send_system(self, send_system).await
    }
}

#[async_trait]
impl Handler<SendPing> for DiscordClient {
    async fn handle(&mut self, send_ping: SendPing, _ctx: &mut XtraContext<Self>) {
        pings::send(self, send_ping).await
    }
}

#[async_trait]
impl Handler<UpdateRelayStatus> for DiscordClient {
    async fn handle(&mut self, update_relay: UpdateRelayStatus, _ctx: &mut XtraContext<Self>) {
        relay::update_status(self, update_relay).await
    }
}

#[async_trait]
impl Handler<ReportError> for DiscordClient {
    async fn handle(&mut self, message: ReportError, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(webhook_config)) =
            (&self.cache_and_http, &self.config.error_webhook)
        {
            if let Ok(webhook) = &cache_and_http
                .http
                .get_webhook_with_token(webhook_config.id, &*webhook_config.token)
                .await
            {
                let embed = Embed::fake(|e| {
                    e.title(message.title);
                    e.description(message.description);
                    if let Some(fields) = message.fields {
                        for (name, value) in fields {
                            e.field(name, value, false);
                        }
                    }
                    e
                });

                if let Err(e) = webhook
                    .execute(&cache_and_http.http, false, |w| {
                        w.embeds(vec![embed]);
                        w.username("Backend error reporting");
                        w
                    })
                    .await
                {
                    warn!("Failed to report error to discord: {}", e);
                }
            } else {
                warn!("Invalid error reporting webhook config!");
            }
        }
    }
}

struct DiscordHandler {
    pings: pings::Handler,
    relay: relay::Handler,
    lfp: lfp::Handler,
}

impl DiscordHandler {
    async fn handle_command(
        &self,
        tokens: &[&str],
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) {
        let admin = check_message_admin(&ctx, &message).await;

        let result = match tokens {
            ["relay", "connect", channel] if admin => {
                self.relay.connect(channel, ctx, message).await
            }
            ["relay", "disconnect"] if admin => self.relay.disconnect(ctx, message).await,
            ["ping", "add", ping, role] if admin => self.pings.add(ctx, message, ping, role).await,
            ["ping", "remove", ping] if admin => self.pings.remove(ctx, message, ping).await,
            ["ping", "allow", ping, role] if admin => {
                self.pings.allow_for_role(ctx, message, ping, role).await
            }
            ["ping", "disallow", ping, role] if admin => {
                self.pings.disallow_for_role(ctx, message, ping, role).await
            }
            ["ping", "request", ping, ..] => self.pings.request(ctx, message, ping).await,
            ["lfp", "setup", ..] => self.lfp.setup_for_channel(ctx, message).await,
            _ => Err(CommandError::InvalidCommand),
        };

        let reaction = if result.is_ok() { '✅' } else { '❌' };
        let _ = message.react(&ctx, reaction).await;

        if let Err(err) = result {
            let _ = message.reply(&ctx, err).await;
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, ctx: SerenityContext, message: SerenityMessage) {
        if !message.author.bot {
            if let Ok(true) = message.mentions_me(&ctx).await {
                let tokens: Vec<&str> = message.content.split_ascii_whitespace().collect();
                self.handle_command(&tokens[1..], &ctx, &message).await;
            } else {
                if message.content.starts_with("//") {
                    self.relay.send_outgoing_command(&ctx, &message).await;
                } else {
                    self.relay.send_outgoing_chat(&ctx, &message).await;
                }
            }
        }
    }

    async fn reaction_add(&self, ctx: SerenityContext, reaction: Reaction) {
        self.lfp.handle_reaction_add(&ctx, reaction).await;
    }

    async fn reaction_remove(&self, ctx: SerenityContext, reaction: Reaction) {
        self.lfp.handle_reaction_remove(&ctx, reaction).await;
    }

    async fn ready(&self, _ctx: SerenityContext, _ready: serenity::model::gateway::Ready) {
        info!("discord bot ready!")
    }
}

async fn check_message_admin(ctx: &SerenityContext, message: &SerenityMessage) -> bool {
    if let Ok(member) = message.member(&ctx).await {
        if let Ok(permissions) = member.permissions(&ctx) {
            return permissions.administrator();
        }
    }
    false
}

pub type CommandResult = std::result::Result<(), CommandError>;

#[derive(thiserror::Error, Debug)]
pub enum CommandError {
    #[error("Discord error!")]
    Serenity(#[from] serenity::Error),
    #[error("This channel is already connected as a relay!")]
    ChannelAlreadyConnected,
    #[error("This channel is not connected as a relay!")]
    ChannelNotConnected,
    #[error("Cannot run this command here!")]
    CannotRunHere,
    #[error("Invalid command!")]
    InvalidCommand,
    #[error("This ping is already connected!")]
    PingAlreadyConnected,
    #[error("This ping is not connected here!")]
    PingNotConnected,
    #[error("Invalid role id!")]
    InvalidRoleId,
    #[error("Please provide changelog in a ```codeblock```")]
    MissingChangelog,
    #[error("You are not allowed to do this!")]
    NotAllowed,
    #[error("You must mention a role with this command!")]
    MustMentionRole,
}
