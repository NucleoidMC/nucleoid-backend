use std::collections::{HashMap, HashSet};
use std::time::{Duration, SystemTime};

use tracing::error;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serenity::all::{CreateAllowedMentions, CreateWebhook};
use serenity::client::Context as SerenityContext;
use serenity::model::channel::{Channel, Message as SerenityMessage};
use serenity::model::id::{ChannelId, RoleId};
use serenity::model::webhook::Webhook;
use serenity::prelude::*;
use xtra::prelude::*;

use crate::{DiscordConfig, Persistent};

use super::*;

pub struct StoreKey;

impl TypeMapKey for StoreKey {
    type Value = Persistent<Store>;
}

#[derive(Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Store {
    pings: HashMap<String, Ping>,
}

impl Store {
    pub fn ping_for_channel(&self, ping: &str, channel: ChannelId) -> Option<&Ping> {
        self.pings
            .get(ping)
            .filter(|ping| ping.discord_channel == channel.get())
    }

    pub fn ping_for_channel_mut(&mut self, ping: &str, channel: ChannelId) -> Option<&mut Ping> {
        self.pings
            .get_mut(ping)
            .filter(|ping| ping.discord_channel == channel.get())
    }
}

#[derive(Serialize, Deserialize)]
pub struct Ping {
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
            _ => false,
        }
    }
}

pub async fn send(discord: &mut DiscordClient, send_ping: SendPing) {
    if let (Some(cache_and_http), Some(data)) =
        (discord.cache_and_http.clone(), discord.data.clone())
    {
        let mut data = data.write().await;
        let ping_store = data.get_mut::<StoreKey>().unwrap();

        // horrible solution to work around not having async closures
        {
            let ping_store = ping_store.get_mut_unchecked();

            if let Some(ping) = ping_store.pings.get_mut(&send_ping.ping) {
                let role = RoleId::new(ping.discord_role);

                let new_ping = ping.try_new_ping(&discord.config);

                let result = ping
                    .webhook
                    .execute(&cache_and_http.http, false, {
                        let mut execute = ExecuteWebhook::new()
                            .allowed_mentions(CreateAllowedMentions::new().roles([role]))
                            .username(send_ping.sender_name)
                            .content(if new_ping {
                                format!("{}! {}", role.mention(), send_ping.content)
                            } else {
                                send_ping.content
                            });
                        if let Some(icon) = send_ping.sender_icon {
                            execute = execute.avatar_url(icon);
                        }
                        execute
                    })
                    .await;

                if let Err(error) = result {
                    error!("failed to send ping: {:?}", error)
                }
            }
        }

        ping_store.flush().await;
    }
}

pub struct Handler {
    pub discord: Address<DiscordClient>,
}

impl Handler {
    pub async fn add(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
        role_id: &str,
    ) -> CommandResult {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<StoreKey>().unwrap();

        let role_id = RoleId::new(
            role_id
                .parse::<u64>()
                .map_err(|_| CommandError::InvalidRoleId)?,
        );

        if let Some(guild) = message.guild(&ctx.cache) {
            if !guild.roles.contains_key(&role_id) {
                return Err(CommandError::InvalidRoleId);
            }
        } else {
            return Err(CommandError::CannotRunHere);
        }

        if let Ok(Channel::Guild(channel)) = message.channel(ctx).await {
            let webhook = channel
                .create_webhook(&ctx.http, CreateWebhook::new(format!("Ping {}", ping)))
                .await?;

            ping_store
                .write(|ping_store| {
                    let ping = ping.to_owned();
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        ping_store.pings.entry(ping)
                    {
                        e.insert(Ping {
                            discord_channel: channel.id.get(),
                            discord_role: role_id.get(),
                            webhook,
                            last_ping_time: SystemTime::now(),
                            allowed_roles: HashSet::new(),
                        });
                        Ok(())
                    } else {
                        Err(CommandError::PingAlreadyConnected)
                    }
                })
                .await
        } else {
            Err(CommandError::CannotRunHere)
        }
    }

    pub async fn remove(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
    ) -> CommandResult {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<StoreKey>().unwrap();

        let ping = ping_store
            .write(|ping_store| {
                if ping_store
                    .ping_for_channel(ping, message.channel_id)
                    .is_none()
                {
                    return Err(CommandError::PingNotConnected);
                }

                match ping_store.pings.remove(ping) {
                    Some(ping) => Ok(ping),
                    None => Err(CommandError::PingNotConnected),
                }
            })
            .await?;

        ping.webhook.delete(&ctx.http).await?;

        Ok(())
    }

    pub async fn allow_for_role(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
        role: &str,
    ) -> CommandResult {
        let role = role
            .parse::<u64>()
            .map_err(|_| CommandError::InvalidRoleId)?;

        {
            let guild = message
                .guild(&ctx.cache)
                .ok_or(CommandError::CannotRunHere)?;
            if !guild.roles.contains_key(&RoleId::new(role)) {
                return Err(CommandError::InvalidRoleId);
            }
        }

        self.update_ping(ctx, message, ping, |ping| {
            if ping.allowed_roles.insert(role) {
                Ok(())
            } else {
                Err(CommandError::InvalidRoleId)
            }
        })
        .await
    }

    pub async fn disallow_for_role(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
        role: &str,
    ) -> CommandResult {
        let role = role
            .parse::<u64>()
            .map_err(|_| CommandError::InvalidRoleId)?;

        self.update_ping(ctx, message, ping, |ping| {
            if ping.allowed_roles.remove(&role) {
                Ok(())
            } else {
                Err(CommandError::InvalidRoleId)
            }
        })
        .await
    }

    async fn update_ping<F>(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
        f: F,
    ) -> CommandResult
    where
        F: FnOnce(&mut Ping) -> CommandResult,
    {
        let mut data = ctx.data.write().await;
        let ping_store = data.get_mut::<StoreKey>().unwrap();

        ping_store
            .write(
                |ping_store| match ping_store.ping_for_channel_mut(ping, message.channel_id) {
                    Some(ping) => f(ping),
                    None => Err(CommandError::PingNotConnected),
                },
            )
            .await
    }

    pub async fn request(
        &self,
        ctx: &SerenityContext,
        message: &SerenityMessage,
        ping: &str,
    ) -> CommandResult {
        let sender_roles = message
            .member
            .as_ref()
            .map(|member| &member.roles)
            .ok_or(CommandError::NotAllowed)?;

        let data = ctx.data.read().await;
        let ping_store = data.get::<StoreKey>().unwrap();

        match ping_store.pings.get(ping) {
            Some(ping) => {
                let allowed = sender_roles
                    .iter()
                    .any(|role| ping.allowed_roles.contains(&role.get()));
                if !allowed {
                    return Err(CommandError::NotAllowed);
                }
            }
            None => return Err(CommandError::PingNotConnected),
        };

        let changelog = Regex::new(r#"(?s)```(.*)```"#).unwrap();
        let changelog = changelog
            .captures(&message.content)
            .and_then(|captures| captures.get(0))
            .map(|changelog| changelog.as_str());

        match changelog {
            Some(changelog) => {
                let _ = self
                    .discord
                    .send(SendPing {
                        ping: ping.to_owned(),
                        sender_name: message.author.name.clone(),
                        sender_icon: message.author.avatar_url().clone(),
                        content: changelog.to_owned(),
                    })
                    .await;
                Ok(())
            }
            None => Err(CommandError::MissingChangelog),
        }
    }
}
