use std::time::{Duration, SystemTime};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serenity::model::channel::{Channel as SerenityChannel, GuildChannel};
use serenity::model::id::{ChannelId, MessageId, RoleId, UserId};
use serenity::model::webhook::Webhook;

use super::*;

// TODO: this code is really, really bad!
//       we can also make improvement to behavior by NOT deleting users' messages until that
//       10 minute time period has passed. That way you cannot hide a ping that you sent.

const REACTION: char = '👋';

pub struct StoreKey;

impl TypeMapKey for StoreKey {
    type Value = Persistent<Store>;
}

#[derive(Serialize, Deserialize, Default)]
pub struct Store {
    channels: ChannelMap,
    last_ping_time: Option<SystemTime>,
}

impl Store {
    fn add(&mut self, channel: ChannelId, role: RoleId, register_message: MessageId, webhook: Webhook) {
        self.channels.0.insert(channel.0, Channel {
            channel_id: channel.0,
            role_id: role.0,
            register_message: register_message.0,
            registrations: Vec::new(),
            webhook,
        });
    }

    fn try_ping(&mut self, config: &DiscordConfig) -> bool {
        let now = SystemTime::now();

        let can_ping = match self.last_ping_time {
            Some(last_ping_time) => {
                let interval = Duration::from_secs(config.lfp_ping_interval_minutes as u64 * 60);
                match now.duration_since(last_ping_time) {
                    Ok(duration) if duration > interval => true,
                    _ => false
                }
            }
            None => true
        };

        if can_ping {
            self.last_ping_time = Some(now);
            true
        } else {
            false
        }
    }
}

#[derive(Default)]
struct ChannelMap(HashMap<u64, Channel>);

impl Serialize for ChannelMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let channels: Vec<_> = self.0.values().collect();
        channels.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ChannelMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let channels = Vec::<Channel>::deserialize(deserializer)?.into_iter()
            .map(|channel| (channel.channel_id, channel))
            .collect();
        Ok(ChannelMap(channels))
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Channel {
    channel_id: u64,
    role_id: u64,
    register_message: u64,
    registrations: Vec<Registration>,
    webhook: Webhook,
}

impl Channel {
    fn add_registration(&mut self, user: UserId, message: MessageId) {
        self.registrations.push(Registration {
            user_id: user.0,
            message_id: message.0,
        });
    }

    fn remove_registration(&mut self, user: UserId) -> Option<Registration> {
        match self.registrations.iter().position(|r| r.user_id == user.0) {
            Some(idx) => Some(self.registrations.remove(idx)),
            None => None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
struct Registration {
    user_id: u64,
    message_id: u64,
}

pub struct Handler {
    pub discord: Address<DiscordClient>,
    pub config: DiscordConfig,
}

impl Handler {
    pub async fn setup_for_channel(&self, ctx: &SerenityContext, message: &SerenityMessage) -> CommandResult {
        let role = message.mention_roles.iter().next().copied()
            .ok_or(CommandError::MustMentionRole)?;

        let description = self.parse_description(&message.content)
            .unwrap_or(format!("Add a {} reaction to register as *looking for players*", REACTION));

        let register_message = message.channel_id.send_message(&ctx.http, |message| {
            message.content(description)
                .reactions(vec![REACTION])
        }).await?;

        let channel = match message.channel(ctx).await {
            Ok(SerenityChannel::Guild(channel)) => channel,
            _ => return Err(CommandError::CannotRunHere),
        };

        let webhook = channel.create_webhook(&ctx.http, format!("Looking For Players")).await?;

        let mut data = ctx.data.write().await;
        let store = data.get_mut::<StoreKey>().unwrap();

        store.write(|store| {
            store.add(message.channel_id, role, register_message.id, webhook);
        }).await;

        Ok(())
    }

    fn parse_description(&self, message: &str) -> Option<String> {
        message.find("\n").map(|idx| message[idx..].to_owned())
    }

    pub async fn handle_reaction_add(&self, ctx: &SerenityContext, reaction: Reaction) {
        if let Some(channel) = self.get_channel(ctx, reaction.channel_id).await {
            let guild_channel = ctx.cache.channel(reaction.channel_id)
                .and_then(|c| c.guild());

            if let (Some(user), Some(guild_channel)) = (reaction.user_id, guild_channel) {
                if let Err(err) = self.add_registration(ctx, user, guild_channel, channel).await {
                    error!("Failed to add looking-for-player registration: {:?}", err);
                }
            }
        }
    }

    async fn add_registration(&self, ctx: &SerenityContext, user: UserId, channel: GuildChannel, channel_data: Channel) -> CommandResult {
        let mut member = channel.guild_id.member(&ctx.http, user).await?;
        if member.user.bot {
            return Ok(());
        }

        member.add_role(&ctx.http, channel_data.role_id).await?;

        let mut data = ctx.data.write().await;
        let store = data.get_mut::<StoreKey>().unwrap();

        let pings = store.write(|store| {
            store.try_ping(&self.config)
        }).await;

        let message = channel_data.webhook.execute(&ctx.http, true, |message| {
            let name = member.nick.clone().unwrap_or(member.user.name.clone());
            let avatar = member.user.avatar_url().unwrap_or(member.user.default_avatar_url());
            let content = if pings {
                format!("{}: {} is looking for players!", RoleId(channel_data.role_id).mention(), member.mention())
            } else {
                format!("{} is looking for players!", member.mention())
            };
            message.content(content)
                .username(name)
                .avatar_url(avatar)
        }).await?;

        if let Some(message) = message {
            store.write(|store| {
                store.channels.0.get_mut(&channel.id.0)
                    .map(|channel| channel.add_registration(user, message.id))
            }).await;
        }

        Ok(())
    }

    pub async fn handle_reaction_remove(&self, ctx: &SerenityContext, reaction: Reaction) {
        if let Some(channel) = self.get_channel(ctx, reaction.channel_id).await {
            let guild_channel = ctx.cache.channel(reaction.channel_id)
                .and_then(|c| c.guild());

            if let (Some(user), Some(guild_channel)) = (reaction.user_id, guild_channel) {
                if let Err(err) = self.remove_registration(ctx, user, guild_channel, channel).await {
                    error!("Failed to remove looking-for-player registration: {:?}", err);
                }
            }
        }
    }

    async fn remove_registration(&self, ctx: &SerenityContext, user: UserId, channel: GuildChannel, channel_data: Channel) -> CommandResult {
        let mut member = channel.guild_id.member(&ctx.http, user).await?;
        if member.user.bot {
            return Ok(());
        }

        member.remove_role(&ctx.http, channel_data.role_id).await?;

        let mut data = ctx.data.write().await;
        let store = data.get_mut::<StoreKey>().unwrap();

        let registration = store.write(|store| {
            store.channels.0.get_mut(&channel.id.0)
                .and_then(|channel| channel.remove_registration(user))
        }).await;

        if let Some(registration) = registration {
            channel.id.delete_message(&ctx.http, registration.message_id).await?;
        }

        Ok(())
    }

    async fn get_channel(&self, ctx: &SerenityContext, channel: ChannelId) -> Option<Channel> {
        let data = ctx.data.read().await;
        let store = data.get::<StoreKey>().unwrap();
        store.channels.0.get(&channel.0).cloned()
    }
}
