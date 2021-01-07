use std::collections::hash_map::Entry;
use std::collections::HashMap;

use async_trait::async_trait;
use xtra::prelude::*;

use crate::Config;
use crate::discord::{self, DiscordClient};
use crate::integrations::{self, IntegrationsClient};
use crate::model::*;
use crate::web;

// TODO: use numerical channel ids internally?
pub struct Controller {
    config: Config,
    discord: Option<Address<DiscordClient>>,
    integration_clients: HashMap<String, Address<IntegrationsClient>>,
    status_by_channel: HashMap<String, ServerStatus>,
}

impl Controller {
    pub fn new(config: Config) -> Controller {
        Controller {
            config,
            discord: None,
            integration_clients: HashMap::new(),
            status_by_channel: HashMap::new(),
        }
    }
}

impl Actor for Controller {}

pub struct RegisterIntegrationsClient {
    pub channel: String,
    pub client: Address<IntegrationsClient>,
}

impl Message for RegisterIntegrationsClient {
    type Result = ();
}

pub struct UnregisterIntegrationsClient {
    pub channel: String,
}

impl Message for UnregisterIntegrationsClient {
    type Result = ();
}

pub struct RegisterDiscordClient {
    pub client: Address<DiscordClient>,
}

impl Message for RegisterDiscordClient {
    type Result = ();
}

pub struct UnregisterDiscordClient;

impl Message for UnregisterDiscordClient {
    type Result = ();
}

pub struct IncomingChat {
    pub channel: String,
    pub sender: Player,
    pub content: String,
}

impl Message for IncomingChat {
    type Result = ();
}

pub struct OutgoingChat {
    pub channel: String,
    pub sender: String,
    pub content: String,
    pub name_color: Option<u32>,
}

impl Message for OutgoingChat {
    type Result = ();
}

pub struct StatusUpdate {
    pub channel: String,
    pub games: Vec<Game>,
    pub players: Vec<Player>,
}

impl Message for StatusUpdate {
    type Result = ();
}

pub struct ServerLifecycleStart {
    pub channel: String,
    pub game_version: String,
}

impl Message for ServerLifecycleStart {
    type Result = ();
}

pub struct ServerLifecycleStop {
    pub channel: String,
}

impl Message for ServerLifecycleStop {
    type Result = ();
}

pub struct GetStatus(pub String);

impl Message for GetStatus {
    type Result = ServerStatus;
}

#[async_trait]
impl Handler<RegisterIntegrationsClient> for Controller {
    async fn handle(&mut self, message: RegisterIntegrationsClient, _ctx: &mut Context<Self>) {
        self.integration_clients.insert(message.channel, message.client);
    }
}

#[async_trait]
impl Handler<UnregisterIntegrationsClient> for Controller {
    async fn handle(&mut self, message: UnregisterIntegrationsClient, _ctx: &mut Context<Self>) {
        self.integration_clients.remove(&message.channel);
    }
}

#[async_trait]
impl Handler<RegisterDiscordClient> for Controller {
    async fn handle(&mut self, message: RegisterDiscordClient, _ctx: &mut Context<Self>) {
        self.discord = Some(message.client);
    }
}

#[async_trait]
impl Handler<UnregisterDiscordClient> for Controller {
    async fn handle(&mut self, _: UnregisterDiscordClient, _ctx: &mut Context<Self>) {
        self.discord.take();
    }
}

#[async_trait]
impl Handler<IncomingChat> for Controller {
    async fn handle(&mut self, message: IncomingChat, _ctx: &mut Context<Self>) {
        println!("[{}] <{}> {}", message.channel, message.sender.name, message.content);

        if let Some(discord) = &self.discord {
            let _ = discord.do_send_async(discord::SendChat {
                channel: message.channel,
                sender: message.sender,
                content: message.content,
            }).await;
        }
    }
}

#[async_trait]
impl Handler<OutgoingChat> for Controller {
    async fn handle(&mut self, message: OutgoingChat, _ctx: &mut Context<Self>) {
        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations.do_send_async(integrations::OutgoingMessage::Chat {
                sender: message.sender,
                content: message.content,
                name_color: message.name_color,
            }).await;
        }
    }
}

#[async_trait]
impl Handler<StatusUpdate> for Controller {
    async fn handle(&mut self, message: StatusUpdate, _ctx: &mut Context<Self>) {
        println!("[{}] {} games, {} players", message.channel, message.games.len(), message.players.len());

        let status = self.status_by_channel.entry(message.channel)
            .or_insert_with(|| ServerStatus::default());
        status.games = message.games;
        status.players = message.players;
    }
}

#[async_trait]
impl Handler<ServerLifecycleStart> for Controller {
    async fn handle(&mut self, message: ServerLifecycleStart, _ctx: &mut Context<Self>) {
        println!("[{}] started @ {}", message.channel, message.game_version);

        if let Some(discord) = &self.discord {
            let _ = discord.do_send_async(discord::SendSystem {
                channel: message.channel.clone(),
                content: format!("Server has started on **{}**!", message.game_version),
            }).await;
        }

        let status = self.status_by_channel.entry(message.channel)
            .or_insert_with(|| ServerStatus::default());
        status.game_version = message.game_version;
    }
}

#[async_trait]
impl Handler<ServerLifecycleStop> for Controller {
    async fn handle(&mut self, message: ServerLifecycleStop, _ctx: &mut Context<Self>) {
        println!("[{}] stopped", message.channel);
        self.status_by_channel.remove(&message.channel);

        if let Some(discord) = &self.discord {
            let _ = discord.do_send_async(discord::SendSystem {
                channel: message.channel,
                content: "Server has stopped!".to_owned(),
            }).await;
        }
    }
}

#[async_trait]
impl Handler<GetStatus> for Controller {
    async fn handle(&mut self, message: GetStatus, _ctx: &mut Context<Self>) -> ServerStatus {
        self.status_by_channel.get(&message.0).cloned()
            .unwrap_or(ServerStatus::default())
    }
}
