use anyhow::Result;
use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::{bson::doc, Client, Collection, Database};
use mongodb::options::FindOptions;
use uuid::Uuid;
use xtra::{Actor, Context, Handler, Message, Address};

use crate::util::uuid_to_bson;
use std::collections::HashMap;
use bson::Document;
use crate::statistics::model::{PlayerProfile, PlayerGameStats, GlobalGameStats, PlayerStatsResponse, GameStatsBundle};
use crate::{StatisticsConfig, Controller, BackendError};

const CORRUPT_STATS_DESCRIPTION: &str = r#"
The backend detected an invalid statistic document while uploading a bundle.
It is likely a minigame has changed the type of one of its stored statistics.
The affected document(s) have been backed up and removed from the database.
"#;

pub struct StatisticDatabaseController {
    controller: Address<Controller>,
    client: Client,
    config: StatisticsConfig,
}

impl StatisticDatabaseController {
    pub async fn connect(controller: &Address<Controller>, config: &StatisticsConfig) -> Result<Self> {
        let handler = Self {
            controller: controller.clone(),
            client: Client::with_uri_str(&*config.database_url).await?,
            config: config.clone(),
        };

        // Ping the database to ensure we can connect and so we crash early if we can't
        handler.client.database("admin")
            .run_command(doc! {"ping": 1}, None)
            .await?;

        Ok(handler)
    }

    fn database(&self) -> Database {
        self.client.database(&*self.config.database_name)
    }

    fn player_profiles(&self) -> Collection<PlayerProfile> {
        self.database().collection("players")
    }

    fn player_stats(&self) -> Collection<PlayerGameStats> {
        self.database().collection("player-stats")
    }

    fn global_stats(&self) -> Collection<GlobalGameStats> {
        self.database().collection("global-stats")
    }

    // Used for error handling
    fn document_player_stats(&self) -> Collection<Document> {
        self.database().collection("player-stats")
    }

    fn document_global_stats(&self) -> Collection<Document> {
        self.database().collection("global-stats")
    }

    fn corrupt_stats(&self) -> Collection<Document> {
        self.database().collection("corrupt_stats")
    }

    async fn get_player_profile(&self, uuid: &Uuid) -> Result<Option<PlayerProfile>> {
        let options = FindOptions::builder().limit(1).build();
        let profile = self.player_profiles()
            .find(doc! {"uuid": uuid_to_bson(uuid)?}, options).await?
            .try_next().await?;
        Ok(profile)
    }

    async fn update_player_profile(&self, uuid: &Uuid, username: Option<String>) -> Result<PlayerProfile> {
        match self.get_player_profile(uuid).await? {
            Some(profile) => {
                if let Some(username) = username {
                    if let Some(profile_username) = profile.username.clone() {
                        if username != profile_username {
                            log::debug!("Player {} updated username to {}", uuid, &username);
                            self.player_profiles().update_one(
                                doc! {"uuid": uuid_to_bson(uuid)?},
                                doc! {"$set": {
                                    "username": username.clone(),
                                }},
                                None,
                            ).await?;

                            let mut profile = profile.clone();
                            profile.username = Some(username.clone());
                            return Ok(profile);
                        }
                    }
                }
                Ok(profile.clone())
            }
            None => {
                let profile = PlayerProfile {
                    uuid: *uuid,
                    username: username.clone(),
                };
                self.player_profiles().insert_one(&profile, None).await?;
                Ok(profile)
            }
        }
    }

    async fn get_player_stats(&self, uuid: &Uuid, namespace: &Option<String>) -> Result<Option<PlayerStatsResponse>> {
        if self.get_player_profile(uuid).await?.is_none() { // player not found.
            return Ok(None);
        }

        let options = FindOptions::builder().build();
        let mut stats = self.player_stats().find(match namespace {
            Some(namespace) => doc! {
                "uuid": uuid_to_bson(uuid)?,
                "namespace": namespace.clone(),
            },
            None => doc! {
                "uuid": uuid_to_bson(uuid)?,
            },
        }, options).await?;

        let mut final_stats: HashMap<String, HashMap<String, f64>> = HashMap::new();
        while let Some(stats) = stats.try_next().await? {
            let mut s = HashMap::new();
            for (name, stat) in stats.stats {
                s.insert(name, stat.into());
            }
            final_stats.insert(stats.namespace, s);
        }

        Ok(Some(final_stats))
    }

    async fn ensure_player_stats_document(&self, uuid: &Uuid, namespace: &str) -> Result<()> {
        self.update_player_profile(uuid, None).await?; // Ensure that the player is tracked in the database.

        let options = FindOptions::builder().limit(1).build();
        let mut res = self.player_stats().find(doc! {
            "uuid": uuid_to_bson(uuid)?,
            "namespace": namespace,
        }, options).await?;
        let stats = res.try_next().await;

        let needs_new_document = match stats {
            Ok(stats) => stats.is_none(),
            Err(e) => {
                self.handle_broken_player_stats_document(&e.into(), uuid, namespace).await?;
                true
            }
        };

        if needs_new_document {
            self.player_stats().insert_one(PlayerGameStats {
                uuid: *uuid,
                namespace: namespace.to_string(),
                stats: HashMap::new(),
            }, None).await?;
        }

        Ok(())
    }

    async fn ensure_global_stats_document(&self, namespace: &str) -> Result<()> {
        let options = FindOptions::builder().limit(1).build();
        let mut res = self.global_stats().find(doc! {
            "namespace": namespace,
        }, options).await?;

        let stats = res.try_next().await;

        let needs_new_document = match stats {
            Ok(stats) => stats.is_none(),
            Err(e) => {
                self.handle_broken_global_stats_document(&e.into(), &namespace).await?;
                true
            }
        };

        if needs_new_document {
            self.global_stats().insert_one(GlobalGameStats {
                namespace: namespace.to_string(),
                stats: HashMap::new(),
            }, None).await?;
        }

        Ok(())
    }

    async fn upload_stats_bundle(&self, bundle: GameStatsBundle) -> Result<()> {
        for (player, stats) in bundle.stats.players {
            // Ensure that there is a document to upload stats to.
            self.ensure_player_stats_document(&player, &bundle.namespace).await?;
            for (stat_name, stat) in stats {
                self.player_stats().update_one(doc! {
                    "uuid": uuid_to_bson(&player)?,
                    "namespace": &bundle.namespace,
                }, stat.create_increment_operation(&stat_name), None).await?;
            }
        }

        if let Some(global) = bundle.stats.global {
            self.ensure_global_stats_document(&bundle.namespace).await?;
            for (stat_name, stat) in global {
                self.global_stats().update_one(doc! {
                    "namespace": &bundle.namespace,
                }, stat.create_increment_operation(&stat_name), None).await?;
            }
        }

        Ok(())
    }

    async fn handle_broken_player_stats_document(&self, e: &anyhow::Error, uuid: &Uuid, namespace: &str) -> Result<()> {
        let doc = self.document_player_stats().find_one(doc! {
            "uuid": uuid_to_bson(uuid)?,
            "namespace": namespace,
        }, None).await?;

        if let Some(doc) = doc {
            self.handle_broken_document(e, &doc, namespace, false).await?;
            self.document_player_stats().delete_one(doc! {
                "_id": doc.get("_id").unwrap(),
            }, None).await?;
        } else {
            // This should never happen
            log::warn!("Missing corrupt document that was there before!? (player: {}, namespace: {})", uuid, namespace);
        }

        Ok(())
    }

    async fn handle_broken_global_stats_document(&self, e: &anyhow::Error, namespace: &str) -> Result<()> {
        let doc = self.document_global_stats().find_one(doc! {
            "namespace": namespace,
        }, None).await?;

        if let Some(doc) = doc {
            self.handle_broken_document(e, &doc, namespace, true).await?;
            self.document_global_stats().delete_one(doc! {
                "_id": doc.get("_id").unwrap(),
            }, None).await?;
        } else {
            // This should never happen
            log::warn!("Missing corrupt document that was there before!? (global; namespace: {})", namespace);
        }

        Ok(())
    }

    async fn handle_broken_document(&self, e: &anyhow::Error, document: &Document, namespace: &str, global: bool) -> Result<()> {
        let mut corrupt_document = document.clone();
        corrupt_document.remove("_id"); // remove the ID so the driver generates a new one when it is re-inserted
        let corrupt_id = self.corrupt_stats().insert_one(document, None).await?.inserted_id;

        log::warn!("Corrupt stats document (not our fault, probably a minigame's)!\nError: {}\nDocument: {}\nNamespace: {}, global: {}", e, document, namespace, global);
        let mut warning_fields: HashMap<String, String> = HashMap::new();
        warning_fields.insert("Statistic namespace".to_string(), namespace.to_string());
        warning_fields.insert("Global statistic?".to_string(), global.to_string());
        warning_fields.insert("Document backup ID".to_string(), corrupt_id.to_string());

        self.controller.send(BackendError {
            title: ":warning: Corrupt stats document".to_string(),
            description: CORRUPT_STATS_DESCRIPTION.to_string(),
            fields: Some(warning_fields),
        }).await.expect("controller disconnected");

        Ok(())
    }
}

impl Actor for StatisticDatabaseController {}

pub struct GetPlayerProfile(pub Uuid);
impl Message for GetPlayerProfile {
    type Result = Result<Option<PlayerProfile>>;
}

#[async_trait]
impl Handler<GetPlayerProfile> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetPlayerProfile, _ctx: &mut Context<Self>) -> <GetPlayerProfile as Message>::Result {
        self.get_player_profile(&message.0).await
    }
}

pub struct UpdatePlayerProfile {
    pub uuid: Uuid,
    pub username: String,
}

impl Message for UpdatePlayerProfile {
    type Result = Result<()>;
}

#[async_trait]
impl Handler<UpdatePlayerProfile> for StatisticDatabaseController {
    async fn handle(&mut self, message: UpdatePlayerProfile, _ctx: &mut Context<Self>) -> <UpdatePlayerProfile as Message>::Result {
        self.update_player_profile(&message.uuid, Some(message.username)).await?;
        Ok(())
    }
}

pub struct GetPlayerStats {
    pub uuid: Uuid,
    pub namespace: Option<String>,
}

impl Message for GetPlayerStats {
    type Result = Result<Option<PlayerStatsResponse>>;
}

#[async_trait]
impl Handler<GetPlayerStats> for StatisticDatabaseController {
    async fn handle(&mut self, message: GetPlayerStats, _ctx: &mut Context<Self>) -> <GetPlayerStats as Message>::Result {
        self.get_player_stats(&message.uuid, &message.namespace).await
    }
}

pub struct UploadStatsBundle(pub GameStatsBundle);

impl Message for UploadStatsBundle {
    type Result = Result<()>;
}

#[async_trait]
impl Handler<UploadStatsBundle> for StatisticDatabaseController {
    async fn handle(&mut self, message: UploadStatsBundle, _ctx: &mut Context<Self>) -> <UploadStatsBundle as Message>::Result {
        self.upload_stats_bundle(message.0).await
    }
}
