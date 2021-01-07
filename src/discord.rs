use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use log::{error, info};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;
use serenity::CacheAndHttp;
use serenity::client::bridge::gateway::GatewayIntents;
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Channel, Message as SerenityMessage, ReactionType};
use serenity::model::id::ChannelId;
use serenity::model::webhook::Webhook;
use serenity::prelude::*;
use xtra::Context as XtraContext;
use xtra::KeepRunning;
use xtra::Message as XtraMessage;
use xtra::prelude::*;

use crate::{Config, Persistent, TokioGlobal};
use crate::controller::*;
use crate::model::*;

struct RelayStateKey;

impl TypeMapKey for RelayStateKey {
    type Value = Persistent<RelayState>;
}

#[derive(Debug, Default)]
struct RelayState {
    channel_to_relay: HashMap<String, ChannelRelay>,
    discord_to_channel: HashMap<u64, String>,
}

impl RelayState {
    pub fn insert_relay(&mut self, channel: String, relay: ChannelRelay) {
        self.discord_to_channel.insert(relay.discord_channel, channel.clone());
        self.channel_to_relay.insert(channel, relay);
    }

    pub fn remove_relay(&mut self, discord: u64) -> Option<(String, ChannelRelay)> {
        match self.discord_to_channel.remove(&discord) {
            Some(channel) => {
                let relay = self.channel_to_relay.remove(&channel);
                relay.map(move |relay| (channel, relay))
            }
            None => None
        }
    }
}

impl Serialize for RelayState {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.channel_to_relay.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RelayState {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let channel_to_relay: HashMap<String, ChannelRelay> = HashMap::deserialize(deserializer)?;
        let mut discord_to_channel = HashMap::new();
        for (channel, relay) in &channel_to_relay {
            discord_to_channel.insert(relay.discord_channel, channel.clone());
        }
        Ok(RelayState { channel_to_relay, discord_to_channel })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChannelRelay {
    discord_channel: u64,
    webhook: Webhook,
}

pub struct DiscordClient {
    controller: Address<Controller>,
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

pub async fn run(controller: Address<Controller>, config: Config) {
    let token = config.discord_token;
    if token.is_empty() {
        return;
    }

    let relay_state = Persistent::open("relay.json").await;
    let actor = DiscordClient {
        controller: controller.clone(),
        cache_and_http: None,
        data: None,
    };
    let address = actor.create(None).spawn(&mut TokioGlobal);

    let mut client = Client::builder(token)
        .event_handler(DiscordHandler { controller: controller.clone() })
        .intents(GatewayIntents::GUILD_MESSAGES | GatewayIntents::GUILDS)
        .await
        .expect("failed to create client");

    {
        let mut data = client.data.write().await;
        data.insert::<RelayStateKey>(relay_state);
    }

    address.do_send_async(Init {
        cache_and_http: client.cache_and_http.clone(),
        data: client.data.clone(),
    }).await.expect("client disconnected");

    controller.do_send_async(RegisterDiscordClient { client: address }).await
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

#[async_trait]
impl Handler<Init> for DiscordClient {
    async fn handle(&mut self, message: Init, _ctx: &mut XtraContext<Self>) {
        self.cache_and_http = Some(message.cache_and_http);
        self.data = Some(message.data);
    }
}

#[async_trait]
impl Handler<SendChat> for DiscordClient {
    async fn handle(&mut self, message: SendChat, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(data)) = (&self.cache_and_http, &self.data) {
            let data = data.read().await;
            let relay_state = data.get::<RelayStateKey>().unwrap();
            if let Some(relay) = relay_state.channel_to_relay.get(&message.channel) {
                let _ = relay.webhook.execute(&cache_and_http.http, false, move |webhook| {
                    webhook.0.insert("allowed_mentions", json!({"parse": []}));
                    // TODO: configurable url
                    webhook.username(message.sender.name)
                        .avatar_url(format!("https://minotar.net/helm/{}/64", message.sender.id.replace("-", "")))
                        .content(message.content)
                }).await;
            }
        }
    }
}

#[async_trait]
impl Handler<SendSystem> for DiscordClient {
    async fn handle(&mut self, message: SendSystem, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(data)) = (&self.cache_and_http, &self.data) {
            let data = data.read().await;
            let relay_state = data.get::<RelayStateKey>().unwrap();
            if let Some(relay) = relay_state.channel_to_relay.get(&message.channel) {
                let _ = ChannelId(relay.discord_channel).send_message(&cache_and_http.http, move |m| {
                    m.content(message.content).allowed_mentions(|m| m.empty_parse())
                }).await;
            }
        }
    }
}

struct DiscordHandler {
    controller: Address<Controller>,
}

impl DiscordHandler {
    async fn handle_command(&self, tokens: &[&str], ctx: &SerenityContext, message: &SerenityMessage) {
        if !check_message_authorized(&ctx, &message).await {
            return;
        }

        let result = match tokens {
            [_, "relay", "connect", channel] => self.connect_relay(channel, ctx, message).await,
            [_, "relay", "disconnect"] => self.disconnect_relay(ctx, message).await,
            [_, "status", "set", channel] => self.set_status_channel(channel, ctx, message).await,
            [_, "status", "remove"] => self.remove_status_channel(ctx, message).await,
            _ => Err(CommandError::InvalidCommand),
        };

        let reaction = if result.is_ok() { "✅" } else { "❌" };
        let _ = message.react(&ctx, ReactionType::Unicode(reaction.to_owned())).await;

        if let Err(err) = result {
            let _ = message.reply(&ctx, err).await;
        }
    }

    async fn connect_relay(&self, channel: &str, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        let mut data = ctx.data.write().await;

        let relay_state = data.get_mut::<RelayStateKey>().unwrap();
        if relay_state.channel_to_relay.contains_key(channel) {
            return Err(CommandError::ChannelAlreadyConnected);
        }

        match message.channel(&ctx.cache).await {
            Some(Channel::Guild(guild_channel)) => {
                let webhook = guild_channel.create_webhook(&ctx.http, format!("Relay ({})", channel)).await?;

                let relay = ChannelRelay {
                    discord_channel: message.channel_id.0,
                    webhook,
                };

                relay_state.write(move |relay_state| {
                    relay_state.insert_relay(channel.to_owned(), relay);
                }).await;

                Ok(())
            }
            _ => Err(CommandError::CannotConnectHere)
        }
    }

    async fn disconnect_relay(&self, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        let mut data = ctx.data.write().await;

        let relay_state = data.get_mut::<RelayStateKey>().unwrap();

        let (_, relay) = relay_state.write(|relay_state| {
            match relay_state.remove_relay(message.channel_id.0) {
                Some(channel) => Ok(channel),
                None => Err(CommandError::ChannelNotConnected),
            }
        }).await?;

        ctx.http.delete_webhook_with_token(relay.webhook.id.0, &relay.webhook.token).await?;

        Ok(())
    }

    async fn set_status_channel(&self, channel: &str, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        // TODO
        Ok(())
    }

    async fn remove_status_channel(&self, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        // TODO
        Ok(())
    }

    async fn send_outgoing_chat(&self, ctx: &SerenityContext, message: &SerenityMessage) {
        let data = ctx.data.read().await;

        let relay_state = data.get::<RelayStateKey>().unwrap();
        if let Some(channel) = relay_state.discord_to_channel.get(&message.channel_id.0) {
            let sender_name = message.author_nick(&ctx).await.unwrap_or(message.author.name.clone());

            let name_color = self.get_sender_name_color(ctx, message).await;

            self.controller.do_send_async(OutgoingChat {
                channel: channel.clone(),
                sender: sender_name,
                content: message.content_safe(&ctx.cache).await,
                name_color,
            }).await.expect("controller disconnected");
        }
    }

    async fn get_sender_name_color(&self, ctx: &SerenityContext, message: &SerenityMessage) -> Option<u32> {
        if let (Some(member), Some(guild)) = (&message.member, message.guild_id) {
            if let Some(guild) = ctx.cache.guild(guild).await {
                return member.roles.iter()
                    .filter_map(|id| guild.roles.get(id))
                    .filter(|role| role.colour.0 != 0)
                    .max_by_key(|role| role.position)
                    .map(|role| role.colour.0);
            }
        }
        None
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, ctx: SerenityContext, message: SerenityMessage) {
        if let Ok(true) = message.mentions_me(&ctx).await {
            let tokens: Vec<&str> = message.content.split_ascii_whitespace().collect();
            self.handle_command(tokens.as_slice(), &ctx, &message).await;
        } else if !message.author.bot {
            self.send_outgoing_chat(&ctx, &message).await;
        }
    }

    async fn ready(&self, _ctx: SerenityContext, _ready: serenity::model::gateway::Ready) {
        info!("discord bot ready!")
    }
}

async fn check_message_authorized(ctx: &SerenityContext, message: &SerenityMessage) -> bool {
    if let Ok(member) = message.member(&ctx).await {
        if let Ok(permissions) = member.permissions(&ctx.cache).await {
            return permissions.administrator();
        }
    }
    false
}

type CommandResult = std::result::Result<(), CommandError>;

#[derive(thiserror::Error, Debug)]
enum CommandError {
    #[error("Discord error!")]
    Serenity(#[from] serenity::Error),
    #[error("This channel is already connected as a relay!")]
    ChannelAlreadyConnected,
    #[error("This channel is not connected as a relay!")]
    ChannelNotConnected,
    #[error("Cannot connect a relay here!")]
    CannotConnectHere,
    #[error("Invalid command!")]
    InvalidCommand,
}
