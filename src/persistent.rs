use std::ops::Deref;
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Serialize};
use tokio::fs::File;
use tokio::prelude::*;

pub struct Persistent<T: Serialize + DeserializeOwned + Default> {
    path: PathBuf,
    inner: T,
}

impl<T: Serialize + DeserializeOwned + Default> Persistent<T> {
    pub async fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();

        let inner = if path.exists() {
            let mut file = File::open(&path).await.expect("failed to open persistent file");

            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).await.expect("failed to load persistent file");

            serde_json::from_slice(&bytes).expect("failed to deserialize persistent file")
        } else {
            T::default()
        };

        Persistent { path, inner }
    }

    #[inline]
    pub async fn write<F, R>(&mut self, f: F) -> R
        where F: FnOnce(&mut T) -> R
    {
        let result = f(&mut self.inner);

        let mut file = File::create(&self.path).await.expect("failed to create persistent file");

        let bytes = serde_json::to_vec(&self.inner).expect("failed to serialize persistent file");
        file.write_all(&bytes).await.expect("failed to write to persistent file");

        result
    }

    #[inline]
    pub fn read(&self) -> &T {
        &self.inner
    }
}

impl<T: Serialize + DeserializeOwned + Default> Deref for Persistent<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.inner
    }
}
