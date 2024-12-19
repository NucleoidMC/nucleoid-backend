use std::collections::HashMap;
use std::sync::Arc;

use log::{error, info, warn};
use serenity::all::{Cache, CreateEmbed, ExecuteWebhook, Http, Webhook};
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Message, Reaction};
use serenity::{async_trait, prelude::*};
use xtra::prelude::*;
use xtra::Context as XtraContext;

use crate::controller::*;
use crate::model::*;
use crate::{DiscordConfig, Persistent};

mod lfp;
mod pings;
mod relay;

#[derive(Clone)]
struct CacheAndHttp {
    http: Arc<Http>,
    cache: Arc<Cache>,
}

impl CacheHttp for CacheAndHttp {
    fn http(&self) -> &Http {
        &self.http
    }
    
    fn cache(&self) -> Option<&Arc<Cache>> {
        Some(&self.cache)
    }
}

impl AsRef<Http> for CacheAndHttp {
    fn as_ref(&self) -> &Http {
        &self.http
    }
}

pub struct DiscordClient {
    controller: Address<Controller>,
    config: DiscordConfig,
    cache_and_http: Option<CacheAndHttp>,
    data: Option<Arc<RwLock<TypeMap>>>,
}

impl Actor for DiscordClient {
    type Stop = ();

    async fn stopped(self) {
        let _ = self.controller.send(UnregisterDiscordClient).await;
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
    let address = xtra::spawn_tokio(actor, Mailbox::unbounded());

    let handler = DiscordHandler {
        pings: pings::Handler {
            discord: address.clone(),
        },
        relay: relay::Handler {
            controller: controller.clone(),
            _discord: address.clone(),
        },
        lfp: lfp::Handler {
            _discord: address.clone(),
            config: config.clone(),
        },
    };

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT
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
        .send(Init {
            cache_and_http: CacheAndHttp {
                cache: client.cache.clone(),
                http: client.http.clone(),
            },
            data: client.data.clone(),
        })
        .await
        .expect("client disconnected");

    controller
        .send(RegisterDiscordClient { client: address })
        .await
        .expect("controller disconnected");

    client.start().await.expect("failed to run client");
}

struct Init {
    cache_and_http: CacheAndHttp,
    data: Arc<RwLock<TypeMap>>,
}

pub struct SendChat {
    pub channel: String,
    pub sender: Player,
    pub content: String,
}

pub struct SendSystem {
    pub channel: String,
    pub content: String,
}

pub struct SendPing {
    pub ping: String,
    pub sender_name: String,
    pub sender_icon: Option<String>,
    pub content: String,
}

pub struct UpdateRelayStatus {
    pub channel: String,
    pub game_version: String,
    pub server_ip: Option<String>,
    pub player_count: usize,
}

pub struct ReportError {
    pub title: String,
    pub description: String,
    pub fields: Option<HashMap<String, String>>,
}

impl Handler<Init> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, message: Init, _ctx: &mut XtraContext<Self>) {
        self.cache_and_http = Some(message.cache_and_http);
        self.data = Some(message.data);
    }
}

impl Handler<SendChat> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, send_chat: SendChat, _ctx: &mut XtraContext<Self>) {
        relay::send_chat(self, send_chat).await
    }
}

impl Handler<SendSystem> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, send_system: SendSystem, _ctx: &mut XtraContext<Self>) {
        relay::send_system(self, send_system).await
    }
}

impl Handler<SendPing> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, send_ping: SendPing, _ctx: &mut XtraContext<Self>) {
        pings::send(self, send_ping).await
    }
}

impl Handler<UpdateRelayStatus> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, update_relay: UpdateRelayStatus, _ctx: &mut XtraContext<Self>) {
        relay::update_status(self, update_relay).await
    }
}

impl Handler<ReportError> for DiscordClient {
    type Return = ();

    async fn handle(&mut self, message: ReportError, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(webhook_config)) =
            (&self.cache_and_http, &self.config.error_webhook)
        {
            if let Ok(webhook) = Webhook::from_id_with_token(cache_and_http, webhook_config.id, &webhook_config.token).await
            {
                let embed = CreateEmbed::new()
                    .title(message.title)
                    .description(message.description)
                    .fields(if let Some(fields) = message.fields {
                        fields.iter().map(|(name, value)| {
                            (name.clone(), value.clone(), false)
                        }).collect::<Vec<_>>()
                    } else {
                        vec![]
                    });

                let res = webhook
                    .execute(&cache_and_http, false, ExecuteWebhook::new()
                        .username("Backend Error Reporting")
                        .embed(embed))
                .await;

                if let Err(e) = res {
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
        message: &Message,
    ) {
        let admin = check_message_admin(ctx, message).await;

        let result = match tokens {
            ["relay", "connect", channel] if admin => {
                self.relay.connect(channel, ctx, message).await
            }
            ["relay", "disconnect"] if admin => self.relay.disconnect(ctx, message).await,
            ["relay", "command", channel, command @ ..] if admin => {
                self.relay
                    .send_relay_command(ctx, message, channel, command)
                    .await
            }
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
            let _ = message.reply(&ctx, err.to_string()).await;
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, ctx: SerenityContext, message: Message) {
        if !message.author.bot {
            if let Ok(true) = message.mentions_me(&ctx).await {
                let tokens: Vec<&str> = message.content.split_ascii_whitespace().collect();
                self.handle_command(&tokens[1..], &ctx, &message).await;
            } else if message.content.starts_with("//") {
                self.relay.send_outgoing_command(&ctx, &message).await;
            } else {
                self.relay.send_outgoing_chat(&ctx, &message).await;
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

async fn check_message_admin(ctx: &SerenityContext, message: &Message) -> bool {
    if let (Ok(channel), Ok(member)) = (message.channel(ctx).await, message.member(&ctx).await) {
        if let Some(guild) = message.guild(&ctx.cache) {
            let permissions = guild.user_permissions_in(&channel.guild().unwrap(), &member);
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
    #[error("The channel with that name does not exist!")]
    ChannelDoesNotExist,
}
