use std::{num::NonZeroUsize, time::Duration};

use lru::LruCache;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use xtra::{Actor, Address, Context, Handler, Mailbox};

const USER_AGENT: &str = "nucleoid-backend (v1, https://github.com/NucleoidMC/nucleoid-backend)";
const MOJANG_PROFILE_URL: &str = "https://sessionserver.mojang.com/session/minecraft/profile";

const CACHE_CLEAR_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Actor)]
pub struct MojangApiClient {
    client: Client,
    username_cache: LruCache<Uuid, String>,
}

impl MojangApiClient {
    pub fn start(cache_size: NonZeroUsize) -> Result<Address<Self>, ClientError> {
        let username_cache = LruCache::new(cache_size);
        let client = Self {
            client: Client::builder().user_agent(USER_AGENT).build()?,
            username_cache,
        };

        let client = xtra::spawn_tokio(client, Mailbox::unbounded());

        let client_weak = client.downgrade();
        // Based on https://github.com/NucleoidMC/player-face-api/blob/main/src/api.rs#L52-L63
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(CACHE_CLEAR_INTERVAL);
            loop {
                interval.tick().await;
                if client_weak.send(ClearCache).await.is_err() {
                    break;
                }
            }
        });

        Ok(client)
    }

    async fn get_username(&mut self, uuid: &Uuid) -> Result<Option<String>, ClientError> {
        if let Some(username) = self.username_cache.get(uuid) {
            Ok(Some(username.clone()))
        } else {
            let response = self
                .client
                .get(format!("{}/{}", MOJANG_PROFILE_URL, uuid))
                .send()
                .await?;
            if response.status().as_u16() == 204 {
                // mojang why don't you just return a 404 here :/
                Ok(None)
            } else {
                let profile = response.json::<ProfileResponse>().await?;
                let username = profile.name;
                self.username_cache.put(*uuid, username.clone());
                Ok(Some(username))
            }
        }
    }
}

pub struct GetPlayerUsername(pub Uuid);

struct ClearCache;

impl Handler<GetPlayerUsername> for MojangApiClient {
    type Return = Result<Option<ProfileResponse>, ClientError>;

    async fn handle(
        &mut self,
        message: GetPlayerUsername,
        _ctx: &mut Context<Self>,
    ) -> Self::Return {
        let username = self.get_username(&message.0).await?;
        Ok(username.map(|username| ProfileResponse {
            id: message.0,
            name: username,
        }))
    }
}

impl Handler<ClearCache> for MojangApiClient {
    type Return = ();

    async fn handle(&mut self, _message: ClearCache, _ctx: &mut Context<Self>) -> Self::Return {
        self.username_cache.clear();
    }
}

#[derive(Deserialize, Serialize)]
pub struct ProfileResponse {
    id: Uuid,
    name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("request error: {0}")]
    ReqwestError(#[from] reqwest::Error),
}
