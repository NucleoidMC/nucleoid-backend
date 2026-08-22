use std::collections::HashMap;
use std::time::SystemTime;

use xtra::prelude::*;

use crate::database::{self, DatabaseClient};
use crate::discord::{self, DiscordClient, ReportError};
use crate::integrations::{self, IntegrationsClient};
use crate::model::*;
use crate::statistics::database::{StatisticDatabaseController, UploadStatsBundle};
use crate::Config;

// TODO: use numerical channel ids internally?
#[derive(Actor)]
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

#[derive(Debug)]
pub struct RegisterIntegrationsClient {
    pub channel: String,
    pub game_version: String,
    pub server_ip: Option<String>,
    pub client: Address<IntegrationsClient>,
}

#[derive(Debug)]
pub struct UnregisterIntegrationsClient {
    pub channel: String,
}

#[derive(Debug)]
pub struct RegisterDiscordClient {
    pub client: Address<DiscordClient>,
}

pub struct UnregisterDiscordClient;

#[derive(Debug)]
pub struct RegisterDatabaseClient {
    pub client: Address<DatabaseClient>,
}

#[derive(Debug)]
pub struct RegisterStatisticsDatabaseController {
    pub controller: Address<StatisticDatabaseController>,
}

#[derive(Debug)]
pub struct GetStatisticsDatabaseController;

#[derive(Debug)]
pub struct IncomingChat {
    pub channel: String,
    pub sender: Player,
    pub content: String,
}

#[derive(Debug)]
pub struct OutgoingChat {
    pub channel: String,
    pub chat: ChatMessage,
}

#[derive(Debug)]
pub struct OutgoingCommand {
    pub channel: String,
    pub sender: String,
    pub command: String,
    pub roles: Vec<String>,
    pub silent: bool,
}

#[derive(Debug)]
pub struct OutgoingServerChange {
    // This should always be sent to a proxy, never a regular server.
    pub channel: String,
    pub player: String,
    pub target_server: String,
}

#[derive(Debug)]
pub struct StatusUpdate {
    pub channel: String,
    pub games: Option<Vec<Game>>,
    pub players: Option<Vec<Player>>,
}

#[derive(Debug)]
pub struct PerformanceUpdate {
    pub channel: String,
    pub performance: ServerPerformance,
}

#[derive(Debug)]
pub struct ServerLifecycleStart {
    pub channel: String,
    pub server_type: ServerType,
}

#[derive(Debug)]
pub struct ServerLifecycleStop {
    pub channel: String,
    pub crash: bool,
    pub server_type: ServerType,
}

#[derive(Debug)]
pub struct ServerSystemMessage {
    pub channel: String,
    pub content: String,
}

#[derive(Debug)]
pub struct GetStatus(pub String);

#[derive(Debug)]
pub struct BackendError {
    pub title: String,
    pub description: String,
    pub fields: Option<HashMap<String, String>>,
}

impl Handler<RegisterIntegrationsClient> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: RegisterIntegrationsClient, _ctx: &mut Context<Self>) {
        tracing::info!(channel = message.channel, "Register integrations client");
        self.integration_clients
            .insert(message.channel.clone(), message.client);

        let status = self.status_by_channel.entry(message.channel).or_default();
        status.game_version = message.game_version;
        status.server_ip = message.server_ip;
    }
}

impl Handler<UnregisterIntegrationsClient> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: UnregisterIntegrationsClient, _ctx: &mut Context<Self>) {
        tracing::info!(channel = message.channel, "Unregister integrations client");
        self.integration_clients.remove(&message.channel);
    }
}

impl Handler<RegisterDiscordClient> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: RegisterDiscordClient, _ctx: &mut Context<Self>) {
        tracing::debug!("Register discord client");
        self.discord = Some(message.client);
    }
}

impl Handler<UnregisterDiscordClient> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, _: UnregisterDiscordClient, _ctx: &mut Context<Self>) {
        tracing::debug!("Unregister discord client");
        self.discord.take();
    }
}

impl Handler<RegisterDatabaseClient> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: RegisterDatabaseClient, _ctx: &mut Context<Self>) {
        tracing::debug!("Register database client");
        self.database = Some(message.client);
    }
}

impl Handler<RegisterStatisticsDatabaseController> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(
        &mut self,
        message: RegisterStatisticsDatabaseController,
        _ctx: &mut Context<Self>,
    ) {
        tracing::debug!("Register statistics database controller");
        self.statistics = Some(message.controller);
    }
}

impl Handler<GetStatisticsDatabaseController> for Controller {
    type Return = Option<Address<StatisticDatabaseController>>;

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(
        &mut self,
        _message: GetStatisticsDatabaseController,
        _ctx: &mut Context<Self>,
    ) -> Self::Return {
        tracing::debug!("Get statistics database controller");
        self.statistics.clone()
    }
}

impl Handler<IncomingChat> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: IncomingChat, _ctx: &mut Context<Self>) {
        tracing::info!(
            channel = message.channel,
            sender = message.sender.name,
            content = message.content,
            "Routing chat (MC->Discord)",
        );

        if let Some(discord) = &self.discord {
            let _ = discord
                .send(discord::SendChat {
                    channel: message.channel,
                    sender: message.sender,
                    content: message.content,
                })
                .await;
        }
    }
}

impl Handler<OutgoingChat> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: OutgoingChat, _ctx: &mut Context<Self>) {
        tracing::info!(
            channel = message.channel,
            sender = message.chat.sender,
            content = message.chat.content,
            "Routing chat (Discord->MC)",
        );

        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .send(integrations::OutgoingMessage::Chat(message.chat))
                .await;
        }
    }
}

impl Handler<OutgoingCommand> for Controller {
    type Return = bool;

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: OutgoingCommand, _ctx: &mut Context<Self>) -> Self::Return {
        tracing::info!(
            channel = message.channel,
            sender = message.sender,
            command = message.command,
            "Routing command (Discord->MC)",
        );

        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .send(integrations::OutgoingMessage::Command {
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

impl Handler<OutgoingServerChange> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(
        &mut self,
        message: OutgoingServerChange,
        _ctx: &mut Context<Self>,
    ) -> Self::Return {
        tracing::info!(
            channel = message.channel,
            player = message.player,
            target_server = message.target_server,
            "Routing player server transfer",
        );
        if let Some(integrations) = self.integration_clients.get(&message.channel) {
            let _ = integrations
                .send(integrations::OutgoingMessage::SendToServer {
                    player: message.player,
                    target_server: message.target_server,
                })
                .await;
        }
    }
}

impl Handler<StatusUpdate> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: StatusUpdate, _ctx: &mut Context<Self>) {
        let status = self
            .status_by_channel
            .entry(message.channel.clone())
            .or_default();

        if let Some(games) = message.games {
            status.games = games;
        }

        if let Some(players) = message.players {
            status.players = players;
        }

        tracing::info!(
            channel = message.channel,
            games = status.games.len(),
            players = status.players.len(),
            "Routing server status update",
        );

        if let Some(discord) = &self.discord {
            let _ = discord
                .send(discord::UpdateRelayStatus {
                    channel: message.channel.clone(),
                    game_version: status.game_version.clone(),
                    server_ip: status.server_ip.clone(),
                    player_count: status.players.len(),
                })
                .await;
        }

        if let Some(database) = &self.database {
            let _ = database
                .send(database::WriteStatus {
                    channel: message.channel.clone(),
                    time: SystemTime::now(),
                    status: status.clone(),
                })
                .await;
        }
    }
}

impl Handler<PerformanceUpdate> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: PerformanceUpdate, _ctx: &mut Context<Self>) {
        tracing::info!("Routing server performance update");
        if let Some(database) = &self.database {
            let _ = database
                .send(database::WritePerformance {
                    channel: message.channel,
                    time: SystemTime::now(),
                    performance: message.performance,
                })
                .await;
        }
    }
}

impl Handler<ServerLifecycleStart> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: ServerLifecycleStart, _ctx: &mut Context<Self>) {
        tracing::info!(channel = message.channel, "Routing server started");

        if let Some(discord) = &self.discord {
            let _ = discord
                .send(discord::SendSystem {
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
            tracing::info!(?kickback, "Triggering kickback");
            if let Some(proxy_client) = self.integration_clients.get(&*kickback.proxy_channel) {
                let _ = proxy_client
                    .send(integrations::OutgoingMessage::SendServerToServer {
                        from_server: kickback.from_server.clone(),
                        to_server: kickback.to_server.clone(),
                    })
                    .await;
            }
        }
    }
}

impl Handler<ServerLifecycleStop> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: ServerLifecycleStop, _ctx: &mut Context<Self>) {
        tracing::info!(channel = message.channel, "Routing server stopped");
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
                .send(discord::SendSystem {
                    channel: message.channel,
                    content: content.to_owned(),
                })
                .await;
        }
    }
}

impl Handler<ServerSystemMessage> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: ServerSystemMessage, _ctx: &mut Context<Self>) {
        tracing::info!(channel = message.channel, content = message.content, "Routing system message");

        if let Some(discord) = &self.discord {
            let _ = discord
                .send(discord::SendSystem {
                    channel: message.channel,
                    content: message.content,
                })
                .await;
        }
    }
}

impl Handler<GetStatus> for Controller {
    type Return = Option<ServerStatus>;

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(
        &mut self,
        message: GetStatus,
        _ctx: &mut Context<Self>,
    ) -> Option<ServerStatus> {
        tracing::info!("Get channel status");
        self.status_by_channel.get(&message.0).cloned()
    }
}

impl Handler<BackendError> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(&mut self, message: BackendError, _ctx: &mut Context<Self>) {
        tracing::info!("Routing backend error");
        if let Some(discord) = &self.discord {
            let _ = discord
                .send(ReportError {
                    title: message.title,
                    description: message.description,
                    fields: message.fields,
                })
                .await;
        }
    }
}

impl Handler<UploadStatsBundle> for Controller {
    type Return = ();

    #[tracing::instrument(skip(self, _ctx))]
    async fn handle(
        &mut self,
        message: UploadStatsBundle,
        _ctx: &mut Context<Self>,
    ) -> Self::Return {
        tracing::info!("Routing statistics bundle");
        if let Some(statistics) = &self.statistics {
            statistics
                .send(message)
                .await
                .expect("statistics controller disconnected")
        }
    }
}
