use std::collections::HashMap;

use lazy_static::lazy_static;
use log::error;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serenity::all::{CreateAllowedMentions, CreateMessage, CreateWebhook, EditChannel};
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Channel, Message as SerenityMessage};
use serenity::model::id::ChannelId;
use serenity::model::webhook::Webhook;
use serenity::prelude::*;
use xtra::prelude::*;

use crate::controller::*;
use crate::Persistent;

use super::*;

pub struct StoreKey;

impl TypeMapKey for StoreKey {
    type Value = Persistent<Store>;
}

#[derive(Debug, Default)]
pub struct Store {
    channel_to_relay: HashMap<String, ChannelRelay>,
    discord_to_channel: HashMap<u64, String>,
}

impl Store {
    pub fn insert_relay(&mut self, channel: String, relay: ChannelRelay) {
        self.discord_to_channel
            .insert(relay.discord_channel, channel.clone());
        self.channel_to_relay.insert(channel, relay);
    }

    pub fn remove_relay(&mut self, discord: u64) -> Option<(String, ChannelRelay)> {
        match self.discord_to_channel.remove(&discord) {
            Some(channel) => {
                let relay = self.channel_to_relay.remove(&channel);
                relay.map(move |relay| (channel, relay))
            }
            None => None,
        }
    }
}

impl Serialize for Store {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.channel_to_relay.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Store {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let channel_to_relay: HashMap<String, ChannelRelay> = HashMap::deserialize(deserializer)?;
        let mut discord_to_channel = HashMap::new();
        for (channel, relay) in &channel_to_relay {
            discord_to_channel.insert(relay.discord_channel, channel.clone());
        }
        Ok(Store {
            channel_to_relay,
            discord_to_channel,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ChannelRelay {
    discord_guild: u64,
    discord_channel: u64,
    webhook: Webhook,
}

pub async fn send_chat(discord: &mut DiscordClient, send_chat: SendChat) {
    if let (Some(cache_and_http), Some(data)) = (&discord.cache_and_http, &discord.data) {
        let data = data.read().await;
        let relay_store = data.get::<StoreKey>().unwrap();
        if let Some(relay) = relay_store.channel_to_relay.get(&send_chat.channel) {
            let avatar_url = &discord.config.player_avatar_url;

            let result = relay
                .webhook
                .execute(&cache_and_http.http, false, {
                    let mut execute = ExecuteWebhook::new()
                        .username(send_chat.sender.name)
                        .content(send_chat.content)
                        .allowed_mentions(CreateAllowedMentions::new());

                    if let Some(avatar_url) = avatar_url {
                        let id = send_chat.sender.id.replace('-', "");
                        let avatar_url = format!("{}/{}", avatar_url, id);
                        execute = execute.avatar_url(avatar_url);
                    }

                    execute
                })
                .await;

            if let Err(error) = result {
                warn!("failed to relay chat message over webhook: {:?}", error);
            }
        }
    }
}

pub async fn send_system(discord: &mut DiscordClient, send_system: SendSystem) {
    if let (Some(cache_and_http), Some(data)) = (&discord.cache_and_http, &discord.data) {
        let data = data.read().await;
        let relay_store = data.get::<StoreKey>().unwrap();
        if let Some(relay) = relay_store.channel_to_relay.get(&send_system.channel) {
            let result = ChannelId::new(relay.discord_channel)
                .send_message(
                    &cache_and_http.http,
                    CreateMessage::new()
                        .content(send_system.content)
                        .allowed_mentions(CreateAllowedMentions::new()),
                )
                .await;

            if let Err(error) = result {
                warn!("failed to send system message: {:?}", error);
            }
        }
    }
}

pub async fn update_status(discord: &mut DiscordClient, update_relay: UpdateRelayStatus) {
    if !discord.config.relay_channel_topic {
        return;
    }

    if let (Some(cache_and_http), Some(data)) = (&discord.cache_and_http, &discord.data) {
        let data = data.read().await;
        let relay_store = data.get::<StoreKey>().unwrap();

        if let Some(relay) = relay_store.channel_to_relay.get(&update_relay.channel) {
            let topic = match update_relay.server_ip {
                Some(ip) => format!(
                    "{} @ {} | {} players online",
                    ip, update_relay.game_version, update_relay.player_count
                ),
                None => format!(
                    "{} | {} players online",
                    update_relay.game_version, update_relay.player_count
                ),
            };

            let edit_result = ChannelId::new(relay.discord_channel)
                .edit(&cache_and_http.http, EditChannel::new().topic(topic))
                .await;

            if let Err(error) = edit_result {
                error!("failed to update channel topic: {:?}", error);
            }
        }
    }
}

pub struct Handler {
    pub controller: Address<Controller>,
    pub _discord: Address<DiscordClient>,
}

impl Handler {
    pub async fn connect(
        &self,
        channel: &str,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> CommandResult {
        let mut data = ctx.data.write().await;

        let relay_store = data.get_mut::<StoreKey>().unwrap();
        if relay_store.channel_to_relay.contains_key(channel) {
            return Err(CommandError::ChannelAlreadyConnected);
        }

        match message.channel(ctx).await {
            Ok(Channel::Guild(guild_channel)) => {
                let webhook = guild_channel
                    .create_webhook(
                        &ctx.http,
                        CreateWebhook::new(format!("Relay ({})", channel)),
                    )
                    .await?;

                let relay = ChannelRelay {
                    discord_channel: message.channel_id.get(),
                    discord_guild: guild_channel.guild_id.get(),
                    webhook,
                };

                relay_store
                    .write(move |relay_store| {
                        relay_store.insert_relay(channel.to_owned(), relay);
                    })
                    .await;

                Ok(())
            }
            _ => Err(CommandError::CannotRunHere),
        }
    }

    pub async fn disconnect(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> CommandResult {
        let mut data = ctx.data.write().await;

        let relay_store = data.get_mut::<StoreKey>().unwrap();

        let (_, relay) = relay_store
            .write(
                |relay_store| match relay_store.remove_relay(message.channel_id.get()) {
                    Some(channel) => Ok(channel),
                    None => Err(CommandError::ChannelNotConnected),
                },
            )
            .await?;

        relay.webhook.delete(&ctx.http).await?;

        Ok(())
    }

    pub async fn send_outgoing_chat(&self, ctx: &SerenityContext, message: &SerenityMessage) {
        let data = ctx.data.read().await;

        let relay_store = data.get::<StoreKey>().unwrap();
        if let Some(channel) = relay_store
            .discord_to_channel
            .get(&message.channel_id.get())
        {
            let message = self.parse_outgoing_chat_with_reply(ctx, message).await;

            self.controller
                .send(OutgoingChat {
                    channel: channel.clone(),
                    chat: message,
                })
                .await
                .expect("controller disconnected");
        }
    }

    pub async fn send_relay_command(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        channel: &str,
        command: &[&str],
    ) -> CommandResult {
        let sender = self.sender_name(ctx, message).await;
        let roles = if let Ok(member) = message.member(&ctx).await {
            member.roles.iter().map(ToString::to_string).collect()
        } else {
            Vec::new()
        };

        let success = self
            .controller
            .send(OutgoingCommand {
                channel: channel.to_string(),
                command: command.join(" "),
                sender,
                roles,
                silent: true,
            })
            .await
            .expect("controller disconnected");

        if success {
            Ok(())
        } else {
            Err(CommandError::ChannelDoesNotExist)
        }
    }

    pub async fn send_outgoing_command(&self, ctx: &SerenityContext, message: &SerenityMessage) {
        let data = ctx.data.read().await;

        let relay_store = data.get::<StoreKey>().unwrap();
        if let Some(channel) = relay_store
            .discord_to_channel
            .get(&message.channel_id.get())
        {
            let command = self.sanitize_message_content(ctx, message).await[2..].to_owned();
            let sender = self.sender_name(ctx, message).await;
            let roles = if let Ok(member) = message.member(&ctx).await {
                member.roles.iter().map(ToString::to_string).collect()
            } else {
                Vec::new()
            };

            self.controller
                .send(OutgoingCommand {
                    channel: channel.clone(),
                    command,
                    sender,
                    roles,
                    silent: false,
                })
                .await
                .expect("controller disconnected");
        }
    }

    async fn parse_outgoing_chat_with_reply(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> ChatMessage {
        let mut chat = self.parse_outgoing_chat(ctx, message).await;

        if let Some(replying_to) = &message.referenced_message {
            let replying_to = self.parse_outgoing_chat(ctx, replying_to).await;
            chat.replying_to = Some(Box::new(replying_to));
        }

        chat
    }

    async fn parse_outgoing_chat(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> ChatMessage {
        let sender = self.sender_name(ctx, message).await;
        let sender_user = DiscordUser {
            id: message.author.id.get(),
            name: message.author.name.clone(),
        };

        let name_color = self.get_sender_name_color(ctx, message).await;

        let content = self.sanitize_message_content(ctx, message).await;

        let attachments = message
            .attachments
            .iter()
            .map(|attachment| ChatAttachment {
                name: attachment.filename.clone(),
                url: attachment.url.clone(),
            })
            .collect();

        ChatMessage {
            sender,
            sender_user,
            content,
            name_color,
            attachments,
            replying_to: None,
        }
    }

    async fn sender_name(&self, ctx: &SerenityContext, message: &SerenityMessage) -> String {
        message
            .author_nick(&ctx)
            .await
            .unwrap_or_else(|| message.author.name.clone())
    }

    async fn sanitize_message_content(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> String {
        lazy_static! {
            static ref CUSTOM_EMOJI_PATTERN: Regex = Regex::new(r#"<:([^>]*>)"#).unwrap();
        }

        let mut content = message.content_safe(&ctx.cache);

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

    async fn get_sender_name_color(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
    ) -> Option<u32> {
        if let (Some(member), Some(guild)) = (&message.member, message.guild_id) {
            if let Some(guild) = ctx.cache.guild(guild) {
                return member
                    .roles
                    .iter()
                    .filter_map(|id| guild.roles.get(id))
                    .filter(|role| role.colour.0 != 0)
                    .max_by_key(|role| role.position)
                    .map(|role| role.colour.0);
            }
        }
        None
    }
}
