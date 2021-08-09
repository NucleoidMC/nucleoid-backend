use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use lazy_static::lazy_static;
use log::{error, info, warn};
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;
use serenity::CacheAndHttp;
use serenity::client::bridge::gateway::GatewayIntents;
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Channel, Message as SerenityMessage, ReactionType, Embed};
use serenity::model::id::{ChannelId, RoleId};
use serenity::model::webhook::Webhook;
use serenity::prelude::*;
use xtra::Context as XtraContext;
use xtra::KeepRunning;
use xtra::Message as XtraMessage;
use xtra::prelude::*;

use crate::{DiscordConfig, Persistent, TokioGlobal};
use crate::controller::*;
use crate::model::*;

struct RelayStoreKey;

impl TypeMapKey for RelayStoreKey {
    type Value = Persistent<RelayStore>;
}

#[derive(Debug, Default)]
struct RelayStore {
    channel_to_relay: HashMap<String, ChannelRelay>,
    discord_to_channel: HashMap<u64, String>,
}

impl RelayStore {
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

impl Serialize for RelayStore {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.channel_to_relay.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RelayStore {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let channel_to_relay: HashMap<String, ChannelRelay> = HashMap::deserialize(deserializer)?;
        let mut discord_to_channel = HashMap::new();
        for (channel, relay) in &channel_to_relay {
            discord_to_channel.insert(relay.discord_channel, channel.clone());
        }
        Ok(RelayStore { channel_to_relay, discord_to_channel })
    }
}

struct PingStoreKey;

impl TypeMapKey for PingStoreKey {
    type Value = Persistent<PingStore>;
}

#[derive(Serialize, Deserialize, Default)]
#[serde(transparent)]
struct PingStore {
    pings: HashMap<String, Ping>,
}

impl PingStore {
    fn ping_for_channel(&self, ping: &str, channel: ChannelId) -> Option<&Ping> {
        self.pings.get(ping).filter(|ping| ping.discord_channel == channel.0)
    }

    fn ping_for_channel_mut(&mut self, ping: &str, channel: ChannelId) -> Option<&mut Ping> {
        self.pings.get_mut(ping).filter(|ping| ping.discord_channel == channel.0)
    }
}

#[derive(Serialize, Deserialize)]
struct Ping {
    discord_channel: u64,
    discord_role: u64,
    webhook: Webhook,
    last_ping_time: SystemTime,
    allowed_roles: HashSet<u64>,
}

impl Ping {
    fn try_new_ping(&mut self, config: &DiscordConfig) -> bool {
        let now = SystemTime::now();
        let interval = Duration::from_secs(config.ping_interval_minutes as u64 * 60);
        match now.duration_since(self.last_ping_time) {
            Ok(duration) if duration > interval => {
                self.last_ping_time = now;
                true
            }
            _ => false
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChannelRelay {
    discord_channel: u64,
    webhook: Webhook,
}

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

    let actor = DiscordClient {
        controller: controller.clone(),
        config: config.clone(),
        cache_and_http: None,
        data: None,
    };
    let address = actor.create(None).spawn(&mut TokioGlobal);

    let handler = DiscordHandler { controller: controller.clone(), discord: address.clone() };

    let mut client = Client::builder(config.token)
        .event_handler(handler)
        .intents(GatewayIntents::GUILD_MESSAGES | GatewayIntents::GUILDS)
        .await
        .expect("failed to create client");

    {
        let mut data = client.data.write().await;
        data.insert::<RelayStoreKey>(relay_store);
        data.insert::<PingStoreKey>(ping_store);
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
        if let (Some(cache_and_http), Some(data)) = (&self.cache_and_http, &self.data) {
            let data = data.read().await;
            let relay_store = data.get::<RelayStoreKey>().unwrap();
            if let Some(relay) = relay_store.channel_to_relay.get(&send_chat.channel) {
                let avatar_url = &self.config.player_avatar_url;

                let result = relay.webhook.execute(&cache_and_http.http, false, move |webhook| {
                    let mut webhook = webhook
                        .username(send_chat.sender.name)
                        .content(send_chat.content);

                    webhook.0.insert("allowed_mentions", json!({"parse": []}));

                    if let Some(avatar_url) = avatar_url {
                        let id = send_chat.sender.id.replace("-", "");
                        let avatar_url = format!("{}/{}", avatar_url, id);
                        webhook = webhook.avatar_url(avatar_url);
                    }

                    webhook
                }).await;

                if let Err(error) = result {
                    warn!("failed to relay chat message over webhook: {:?}", error);
                }
            }
        }
    }
}

#[async_trait]
impl Handler<SendSystem> for DiscordClient {
    async fn handle(&mut self, send_system: SendSystem, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(data)) = (&self.cache_and_http, &self.data) {
            let data = data.read().await;
            let relay_store = data.get::<RelayStoreKey>().unwrap();
            if let Some(relay) = relay_store.channel_to_relay.get(&send_system.channel) {
                let result = ChannelId(relay.discord_channel).send_message(&cache_and_http.http, move |message| {
                    message.content(send_system.content).allowed_mentions(|m| m.empty_parse())
                }).await;

                if let Err(error) = result {
                    warn!("failed to send system message: {:?}", error);
                }
            }
        }
    }
}

#[async_trait]
impl Handler<SendPing> for DiscordClient {
    async fn handle(&mut self, send_ping: SendPing, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(data)) = (self.cache_and_http.clone(), self.data.clone()) {
            let mut data = data.write().await;
            let ping_store = data.get_mut::<PingStoreKey>().unwrap();

            // horrible solution to work around not having async closures
            {
                let ping_store = ping_store.get_mut_unchecked();

                if let Some(ping) = ping_store.pings.get_mut(&send_ping.ping) {
                    let role = RoleId(ping.discord_role);

                    let new_ping = ping.try_new_ping(&self.config);

                    let result = ping.webhook.execute(&cache_and_http.http, false, move |message| {
                        message.0.insert("allowed_mentions", json!({"parse": [], "roles": [role.to_string()]}));
                        message.username(send_ping.sender_name);

                        if let Some(icon) = send_ping.sender_icon {
                            message.avatar_url(icon);
                        }

                        let content = if new_ping {
                            format!("{}! {}", role.mention(), send_ping.content)
                        } else {
                            send_ping.content
                        };
                        message.content(content)
                    }).await;

                    if let Err(error) = result {
                        error!("failed to send ping: {:?}", error)
                    }
                }
            }

            ping_store.flush().await;
        }
    }
}

#[async_trait]
impl Handler<UpdateRelayStatus> for DiscordClient {
    async fn handle(&mut self, update_relay: UpdateRelayStatus, _ctx: &mut XtraContext<Self>) {
        if !self.config.relay_channel_topic {
            return;
        }

        if let (Some(cache_and_http), Some(data)) = (&self.cache_and_http, &self.data) {
            let data = data.read().await;
            let relay_store = data.get::<RelayStoreKey>().unwrap();

            if let Some(relay) = relay_store.channel_to_relay.get(&update_relay.channel) {
                let topic = match update_relay.server_ip {
                    Some(ip) => format!("{} @ {} | {} players online", ip, update_relay.game_version, update_relay.player_count),
                    None => format!("{} | {} players online", update_relay.game_version, update_relay.player_count)
                };

                if let Some(Channel::Guild(channel)) = cache_and_http.cache.channel(relay.discord_channel).await {
                    if channel.topic.as_ref() == Some(&topic) {
                        return;
                    }
                }

                let edit_result = ChannelId(relay.discord_channel)
                    .edit(&cache_and_http.http, move |channel| {
                        channel.topic(topic)
                    }).await;

                if let Err(error) = edit_result {
                    error!("failed to update channel topic: {:?}", error);
                }
            }
        }
    }
}

#[async_trait]
impl Handler<ReportError> for DiscordClient {
    async fn handle(&mut self, message: ReportError, _ctx: &mut XtraContext<Self>) {
        if let (Some(cache_and_http), Some(webhook_config)) = (&self.cache_and_http, &self.config.error_webhook) {
            if let Ok(webhook) = &cache_and_http.http.get_webhook_with_token(webhook_config.id, &*webhook_config.token).await {
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

                if let Err(e) = webhook.execute(&cache_and_http.http, false, |w| {
                    w.embeds(vec![embed]);
                    w.username("Backend error reporting");
                    w
                }).await {
                    warn!("Failed to report error to discord: {}", e);
                }
            } else {
                warn!("Invalid error reporting webhook config!");
            }
        }
    }
}

struct DiscordHandler {
    controller: Address<Controller>,
    discord: Address<DiscordClient>,
}

impl DiscordHandler {
    async fn handle_command(&self, tokens: &[&str], ctx: &SerenityContext, message: &SerenityMessage) {
        let admin = check_message_admin(&ctx, &message).await;

        let result = match tokens {
            ["relay", "connect", channel] if admin => self.connect_relay(channel, ctx, message).await,
            ["relay", "disconnect"] if admin => self.disconnect_relay(ctx, message).await,
            ["ping", "add", ping, role] if admin => self.add_ping(ctx, message, ping, role).await,
            ["ping", "remove", ping] if admin => self.remove_ping(ctx, message, ping).await,
            ["ping", "allow", ping, role] if admin => self.allow_ping_for_role(ctx, message, ping, role).await,
            ["ping", "disallow", ping, role] if admin => self.disallow_ping_for_role(ctx, message, ping, role).await,
            ["ping", "request", ping, ..] => self.request_ping(ctx, message, ping).await,
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

        let relay_store = data.get_mut::<RelayStoreKey>().unwrap();
        if relay_store.channel_to_relay.contains_key(channel) {
            return Err(CommandError::ChannelAlreadyConnected);
        }

        match message.channel(&ctx.cache).await {
            Some(Channel::Guild(guild_channel)) => {
                let webhook = guild_channel.create_webhook(&ctx.http, format!("Relay ({})", channel)).await?;

                let relay = ChannelRelay {
                    discord_channel: message.channel_id.0,
                    webhook,
                };

                relay_store.write(move |relay_store| {
                    relay_store.insert_relay(channel.to_owned(), relay);
                }).await;

                Ok(())
            }
            _ => Err(CommandError::CannotRunHere)
        }
    }

    async fn disconnect_relay(&self, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        let mut data = ctx.data.write().await;

        let relay_store = data.get_mut::<RelayStoreKey>().unwrap();

        let (_, relay) = relay_store.write(|relay_store| {
            match relay_store.remove_relay(message.channel_id.0) {
                Some(channel) => Ok(channel),
                None => Err(CommandError::ChannelNotConnected),
            }
        }).await?;

        ctx.http.delete_webhook_with_token(relay.webhook.id.0, &relay.webhook.token).await?;

        Ok(())
    }

    async fn add_ping(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str, role_id: &str) -> CommandResult {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<PingStoreKey>().unwrap();

        match (message.guild(&ctx.cache).await, message.channel(&ctx.cache).await) {
            (Some(guild), Some(Channel::Guild(channel))) => {
                let role_id = RoleId(role_id.parse::<u64>().map_err(|_| CommandError::InvalidRoleId)?);
                if !guild.roles.contains_key(&role_id) {
                    return Err(CommandError::InvalidRoleId);
                }

                let webhook = channel.create_webhook(&ctx.http, format!("Ping {}", ping)).await?;

                ping_store.write(|ping_store| {
                    let ping = ping.to_owned();
                    if !ping_store.pings.contains_key(&ping) {
                        ping_store.pings.insert(ping, Ping {
                            discord_channel: channel.id.0,
                            discord_role: role_id.0,
                            webhook,
                            last_ping_time: SystemTime::now(),
                            allowed_roles: HashSet::new(),
                        });
                        Ok(())
                    } else {
                        Err(CommandError::PingAlreadyConnected)
                    }
                }).await
            }
            _ => Err(CommandError::CannotRunHere)
        }
    }

    async fn remove_ping(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str) -> CommandResult {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<PingStoreKey>().unwrap();

        let ping = ping_store.write(|ping_store| {
            if ping_store.ping_for_channel(ping, message.channel_id).is_none() {
                return Err(CommandError::PingNotConnected);
            }

            match ping_store.pings.remove(ping) {
                Some(ping) => Ok(ping),
                None => Err(CommandError::PingNotConnected)
            }
        }).await?;

        let webhook = ping.webhook;
        ctx.http.delete_webhook_with_token(webhook.id.0, &webhook.token).await?;

        Ok(())
    }

    async fn allow_ping_for_role(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str, role: &str) -> CommandResult {
        let role = role.parse::<u64>().map_err(|_| CommandError::InvalidRoleId)?;

        let guild = message.guild(&ctx.cache).await.ok_or(CommandError::CannotRunHere)?;
        if guild.roles.get(&RoleId(role)).is_none() {
            return Err(CommandError::InvalidRoleId);
        }

        self.update_ping(ctx, message, ping, |ping| {
            if ping.allowed_roles.insert(role) {
                Ok(())
            } else {
                Err(CommandError::InvalidRoleId)
            }
        }).await
    }

    async fn disallow_ping_for_role(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str, role: &str) -> CommandResult {
        let role = role.parse::<u64>().map_err(|_| CommandError::InvalidRoleId)?;

        self.update_ping(ctx, message, ping, |ping| {
            if ping.allowed_roles.remove(&role) {
                Ok(())
            } else {
                Err(CommandError::InvalidRoleId)
            }
        }).await
    }

    async fn update_ping<F>(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str, f: F) -> CommandResult
        where F: FnOnce(&mut Ping) -> CommandResult,
    {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<PingStoreKey>().unwrap();

        ping_store.write(|ping_store| {
            match ping_store.ping_for_channel_mut(ping, message.channel_id) {
                Some(ping) => f(ping),
                None => Err(CommandError::PingNotConnected),
            }
        }).await
    }

    async fn request_ping(&self, ctx: &SerenityContext, message: &SerenityMessage, ping: &str) -> CommandResult {
        let sender_roles = message.member.as_ref()
            .map(|member| &member.roles)
            .ok_or(CommandError::NotAllowed)?;

        let data = ctx.data.read().await;
        let ping_store = data.get::<PingStoreKey>().unwrap();

        match ping_store.pings.get(ping) {
            Some(ping) => {
                let allowed = sender_roles.iter()
                    .any(|role| ping.allowed_roles.contains(&role.0));
                if !allowed {
                    return Err(CommandError::NotAllowed);
                }
            }
            None => return Err(CommandError::PingNotConnected),
        };

        let changelog = Regex::new(r#"(?s)```(.*)```"#).unwrap();
        let changelog = changelog.captures(&message.content)
            .and_then(|captures| captures.get(0))
            .map(|changelog| changelog.as_str());

        match changelog {
            Some(changelog) => {
                let _ = self.discord.do_send_async(SendPing {
                    ping: ping.to_owned(),
                    sender_name: message.author.name.clone(),
                    sender_icon: message.author.avatar_url().clone(),
                    content: changelog.to_owned(),
                }).await;
                Ok(())
            }
            None => Err(CommandError::MissingChangelog)
        }
    }

    async fn send_outgoing_chat(&self, ctx: &SerenityContext, message: &SerenityMessage) {
        let data = ctx.data.read().await;

        let relay_store = data.get::<RelayStoreKey>().unwrap();
        if let Some(channel) = relay_store.discord_to_channel.get(&message.channel_id.0) {
            let message = self.parse_outgoing_chat_with_reply(ctx, message).await;

            self.controller.do_send_async(OutgoingChat {
                channel: channel.clone(),
                chat: message,
            }).await.expect("controller disconnected");
        }
    }

    async fn send_outgoing_command(&self, ctx: &SerenityContext, message: &SerenityMessage) {
        let data = ctx.data.read().await;

        let relay_store = data.get::<RelayStoreKey>().unwrap();
        if let Some(channel) = relay_store.discord_to_channel.get(&message.channel_id.0) {
            let command = self.sanitize_message_content(ctx, message).await[2..].to_owned();
            let sender = self.sender_name(ctx, message).await;

            self.controller.do_send_async(OutgoingCommand {
                channel: channel.clone(),
                command,
                sender,
            }).await.expect("controller disconnected");
        }
    }

    async fn parse_outgoing_chat_with_reply(&self, ctx: &SerenityContext, message: &SerenityMessage) -> ChatMessage {
        let mut chat = self.parse_outgoing_chat(ctx, message).await;

        if let Some(replying_to) = &message.referenced_message {
            let replying_to = self.parse_outgoing_chat(ctx, &*replying_to).await;
            chat.replying_to = Some(Box::new(replying_to));
        }

        chat
    }

    async fn parse_outgoing_chat(&self, ctx: &SerenityContext, message: &SerenityMessage) -> ChatMessage {
        let sender = self.sender_name(ctx, message).await;
        let sender_user = DiscordUser {
            id: message.author.id.0,
            name: message.author.name.clone(),
            discriminator: message.author.discriminator,
        };

        let name_color = self.get_sender_name_color(ctx, message).await;

        let content = self.sanitize_message_content(ctx, message).await;

        let attachments = message.attachments.iter()
            .map(|attachment| ChatAttachment {
                name: attachment.filename.clone(),
                url: attachment.url.clone(),
            })
            .collect();

        ChatMessage { sender, sender_user, content, name_color, attachments, replying_to: None }
    }

    async fn sender_name(&self, ctx: &SerenityContext, message: &SerenityMessage) -> String {
        message.author_nick(&ctx).await.unwrap_or(message.author.name.clone())
    }

    async fn sanitize_message_content(&self, ctx: &SerenityContext, message: &SerenityMessage) -> String {
        lazy_static! {
            static ref CUSTOM_EMOJI_PATTERN: Regex = Regex::new(r#"<:([^>]*>)"#).unwrap();
        }

        let mut content = message.content_safe(&ctx.cache).await;

        let mut index = 0;
        while let Some(emoji) = CUSTOM_EMOJI_PATTERN.find_at(&content, index) {
            let range = emoji.range();
            if let Some(emoji) = serenity::utils::parse_emoji(emoji.as_str()) {
                let sanitized_emoji = format!(":{}:", emoji.name);
                index = range.start + sanitized_emoji.len();
                content.replace_range(range, &sanitized_emoji);
            }
        }

        content
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
            self.handle_command(&tokens[1..], &ctx, &message).await;
        } else if !message.author.bot {
            if message.content.starts_with("//") && check_message_admin(&ctx, &message).await {
                self.send_outgoing_command(&ctx, &message).await;
            } else {
                self.send_outgoing_chat(&ctx, &message).await;
            }
        }
    }

    async fn ready(&self, _ctx: SerenityContext, _ready: serenity::model::gateway::Ready) {
        info!("discord bot ready!")
    }
}

async fn check_message_admin(ctx: &SerenityContext, message: &SerenityMessage) -> bool {
    if let Ok(member) = message.member(&ctx).await {
        if let Ok(permissions) = member.permissions(&ctx).await {
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
}
