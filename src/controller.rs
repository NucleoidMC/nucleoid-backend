use std::collections::HashMap;
use std::time::SystemTime;

use async_trait::async_trait;
use xtra::prelude::*;

use crate::database::{self, DatabaseClient};
use crate::discord::{self, DiscordClient, ReportError};
use crate::integrations::{self, IntegrationsClient};
use crate::model::*;
use crate::statistics::database::{StatisticDatabaseController, UploadStatsBundle};
use crate::Config;

// TODO: use numerical channel ids internally?
pub struct Controller {
    config: Config,
    discord: Option<Address<DiscordClient>>,
    database: Option<Address<DatabaseClient>>,
    statistics: Option<Address<StatisticDatabaseController>>,
    integration_clients: HashMap<String, Address<IntegrationsClient>>,
    status_by_channel: HashMap<String, ServerStatus>,
}

impl Controller {
    pub async fn new(config: Config) -> Controller {
        Controller {
            config,
            discord: None,
            database: None,
            statistics: None,
            integration_clients: HashMap::new(),
            status_by_channel: HashMap::new(),
        }
    }
}

impl Actor for Controller {}

pub struct RegisterIntegrationsClient {
    pub channel: String,
    pub game_version: String,
    pub server_ip: Option<String>,
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

pub struct RegisterDatabaseClient {
    pub client: Address<DatabaseClient>,
}

impl Message for RegisterDatabaseClient {
    type Result = ();
}

pub struct RegisterStatisticsDatabaseController {
    pub controller: Address<StatisticDatabaseController>,
}

impl Message for RegisterStatisticsDatabaseController {
    type Result = ();
}

pub struct GetStatisticsDatabaseController;

impl Message for GetStatisticsDatabaseController {
    type Result = Option<Address<StatisticDatabaseController>>;
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
    pub chat: ChatMessage,
}

impl Message for OutgoingChat {
    type Result = ();
}

pub struct OutgoingCommand {
    pub channel: String,
    pub sender: String,
    pub command: String,
    pub roles: Vec<String>,
    pub silent: bool,
}

impl Message for OutgoingCommand {
    type Result = bool;
}

pub struct OutgoingServerChange {
    // This should always be sent to a proxy, never a regular server.
    pub channel: String,
    pub player: String,
    pub target_server: String,
}

impl Message for OutgoingServerChange {
    type Result = ();
}

pub struct StatusUpdate {
    pub channel: String,
    pub games: Option<Vec<Game>>,
    pub players: Option<Vec<Player>>,
}

impl Message for StatusUpdate {
    type Result = ();
}

pub struct PerformanceUpdate {
    pub channel: String,
    pub performance: ServerPerformance,
}

impl Message for PerformanceUpdate {
    type Result = ();
}

pub struct ServerLifecycleStart {
    pub channel: String,
    pub server_type: ServerType,
}

impl Message for ServerLifecycleStart {
    type Result = ();
}

pub struct ServerLifecycleStop {
    pub channel: String,
    pub crash: bool,
    pub server_type: ServerType,
}

impl Message for ServerLifecycleStop {
    type Result = ();
}

pub struct ServerSystemMessage {
    pub channel: String,
    pub content: String,
}

impl Message for ServerSystemMessage {
    type Result = ();
}

pub struct GetStatus(pub String);

impl Message for GetStatus {
    type Result = Option<ServerStatus>;
}

pub struct BackendError {
    pub title: String,
    pub description: String,
    pub fields: Option<HashMap<String, String>>,
}

impl Message for BackendError {
    type Result = ();
}

#[async_trait]
impl Handler<RegisterIntegrationsClient> for Controller {
    async fn handle(&mut self, message: RegisterIntegrationsClient, _ctx: &mut Context<Self>) {
        self.integration_clients
            .insert(message.channel.clone(), message.client);

        let status = self
            .status_by_channel
            .entry(message.channel)
            .or_insert_with(ServerStatus::default);
        status.game_version = message.game_version;
        status.server_ip = message.server_ip;
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
impl Handler<RegisterDatabaseClient> for Controller {
    async fn handle(&mut self, message: RegisterDatabaseClient, _ctx: &mut Context<Self>) {
        self.database = Some(message.client);
    }
}

#[async_trait]
impl Handler<RegisterStatisticsDatabaseController> for Controller {
    async fn handle(
        &mut self,
        message: RegisterStatisticsDatabaseController,
        _ctx: &mut Context<Self>,
    ) {
        self.statistics = Some(message.controller);
    }
}

#[async_trait]
impl Handler<GetStatisticsDatabaseController> for Controller {
    async fn handle(
        &mut self,
        _message: GetStatisticsDatabaseController,
        _ctx: &mut Context<Self>,
    ) -> <GetStatisticsDatabaseController as Message>::Result {
        self.statistics.clone()
    }
}

#[async_trait]
impl Handler<IncomingChat> for Controller {
    async fn handle(&mut self, message: IncomingChat, _ctx: &mut Context<Self>) {
        println!(
            "[{}] <{}> {}",
            message.channel, message.sender.name, message.content
        );

        if let Some(discord) = &self.discord {
            let _ = discord
                .do_send_async(discord::SendChat {
                    channel: message.channel,
                    sender: message.sender,
                    content: message.content,
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<OutgoingChat> for Controller {
    async fn handle(&mut self, message: OutgoingChat, _ctx: &mut Context<Self>) {
        println!(
            "[{}] <@{}> {}",
            message.channel, message.chat.sender, message.chat.content
        );

        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .do_send_async(integrations::OutgoingMessage::Chat(message.chat))
                .await;
        }
    }
}

#[async_trait]
impl Handler<OutgoingCommand> for Controller {
    async fn handle(&mut self, message: OutgoingCommand, _ctx: &mut Context<Self>) -> <OutgoingCommand as Message>::Result {
        println!(
            "[{}] <@{}> /{}",
            message.channel, message.sender, message.command
        );

        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .do_send_async(integrations::OutgoingMessage::Command {
                    command: message.command,
                    sender: message.sender,
                    roles: message.roles,
                    silent: message.silent,
                })
                .await;
            true
        } else {
            false
        }
    }
}

#[async_trait]
impl Handler<OutgoingServerChange> for Controller {
    async fn handle(
        &mut self,
        message: OutgoingServerChange,
        _ctx: &mut Context<Self>,
    ) -> <OutgoingServerChange as Message>::Result {
        println!(
            "[{}] {} -> {}",
            message.channel, message.player, message.target_server
        );
        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .do_send_async(integrations::OutgoingMessage::SendToServer {
                    player: message.player,
                    target_server: message.target_server,
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<StatusUpdate> for Controller {
    async fn handle(&mut self, message: StatusUpdate, _ctx: &mut Context<Self>) {
        let status = self
            .status_by_channel
            .entry(message.channel.clone())
            .or_insert_with(ServerStatus::default);

        if let Some(games) = message.games {
            status.games = games;
        }

        if let Some(players) = message.players {
            status.players = players;
        }

        println!(
            "[{}] {} games, {} players",
            message.channel,
            status.games.len(),
            status.players.len()
        );

        if let Some(discord) = &self.discord {
            let _ = discord
                .do_send_async(discord::UpdateRelayStatus {
                    channel: message.channel.clone(),
                    game_version: status.game_version.clone(),
                    server_ip: status.server_ip.clone(),
                    player_count: status.players.len(),
                })
                .await;
        }

        if let Some(database) = &self.database {
            let _ = database
                .do_send_async(database::WriteStatus {
                    channel: message.channel.clone(),
                    time: SystemTime::now(),
                    status: status.clone(),
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<PerformanceUpdate> for Controller {
    async fn handle(&mut self, message: PerformanceUpdate, _ctx: &mut Context<Self>) {
        if let Some(database) = &self.database {
            let _ = database
                .do_send_async(database::WritePerformance {
                    channel: message.channel,
                    time: SystemTime::now(),
                    performance: message.performance,
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<ServerLifecycleStart> for Controller {
    async fn handle(&mut self, message: ServerLifecycleStart, _ctx: &mut Context<Self>) {
        println!("[{}] started", message.channel);

        if let Some(discord) = &self.discord {
            let _ = discord
                .do_send_async(discord::SendSystem {
                    channel: message.channel.clone(),
                    content: format!(
                        "{} has started!",
                        match message.server_type {
                            ServerType::Minecraft => "Server",
                            ServerType::Velocity => "Proxy",
                        }
                    ),
                })
                .await;
        }

        if let Some(kickback) = self.config.kickbacks.get(&*message.channel) {
            if let Some(proxy_client) = self.integration_clients.get(&*kickback.proxy_channel) {
                let _ = proxy_client
                    .do_send_async(integrations::OutgoingMessage::SendServerToServer {
                        from_server: kickback.from_server.clone(),
                        to_server: kickback.to_server.clone(),
                    })
                    .await;
            }
        }
    }
}

#[async_trait]
impl Handler<ServerLifecycleStop> for Controller {
    async fn handle(&mut self, message: ServerLifecycleStop, _ctx: &mut Context<Self>) {
        println!("[{}] stopped", message.channel);
        self.status_by_channel.remove(&message.channel);

        if let Some(discord) = &self.discord {
            let content = if message.crash {
                format!(
                    "{} has crashed!",
                    match message.server_type {
                        ServerType::Minecraft => "Server",
                        ServerType::Velocity => "Proxy",
                    }
                )
            } else {
                format!(
                    "{} has stopped!",
                    match message.server_type {
                        ServerType::Minecraft => "Server",
                        ServerType::Velocity => "Proxy",
                    }
                )
            };

            let _ = discord
                .do_send_async(discord::SendSystem {
                    channel: message.channel,
                    content: content.to_owned(),
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<ServerSystemMessage> for Controller {
    async fn handle(&mut self, message: ServerSystemMessage, _ctx: &mut Context<Self>) {
        println!("[{}] {}", message.channel, message.content);

        if let Some(discord) = &self.discord {
            let _ = discord
                .do_send_async(discord::SendSystem {
                    channel: message.channel,
                    content: message.content,
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<GetStatus> for Controller {
    async fn handle(
        &mut self,
        message: GetStatus,
        _ctx: &mut Context<Self>,
    ) -> Option<ServerStatus> {
        self.status_by_channel.get(&message.0).cloned()
    }
}

#[async_trait]
impl Handler<BackendError> for Controller {
    async fn handle(&mut self, message: BackendError, _ctx: &mut Context<Self>) {
        if let Some(discord) = &self.discord {
            let _ = discord
                .do_send_async(ReportError {
                    title: message.title,
                    description: message.description,
                    fields: message.fields,
                })
                .await;
        }
    }
}

#[async_trait]
impl Handler<UploadStatsBundle> for Controller {
    async fn handle(
        &mut self,
        message: UploadStatsBundle,
        _ctx: &mut Context<Self>,
    ) -> <UploadStatsBundle as Message>::Result {
        if let Some(statistics) = &self.statistics {
            statistics
                .send(message)
                .await
                .expect("statistics controller disconnected")
        }
    }
}
