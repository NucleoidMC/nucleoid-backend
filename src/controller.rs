use std::collections::HashMap;

use async_trait::async_trait;
use xtra::prelude::*;

use crate::Config;
use crate::discord::{self, DiscordClient};
use crate::integrations::{self, IntegrationsClient};
use crate::model::*;

// TODO: use numerical channel ids internally?
pub struct Controller {
    config: Config,
    discord: Option<Address<DiscordClient>>,
    integration_clients: HashMap<String, Address<IntegrationsClient>>,
}

impl Controller {
    pub fn new(config: Config) -> Controller {
        Controller {
            config,
            discord: None,
            integration_clients: HashMap::new(),
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
                content: message.content
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
            }).await;
        }
    }
}

#[async_trait]
impl Handler<StatusUpdate> for Controller {
    async fn handle(&mut self, message: StatusUpdate, _ctx: &mut Context<Self>) {
        println!("[{}] {} games, {} players", message.channel, message.games.len(), message.players.len());
    }
}
