use std::sync::Arc;

use log::debug;
use tokio::fs::File;
use tokio::io::AsyncReadExt;
use tokio::io::Error;

use tokio::sync::Mutex;
use tokio::sync::OwnedMutexGuard;

#[derive(Clone)]
pub struct FilePool {
    path: String,
    max_readers: usize,
    writer: Arc<Mutex<File>>,
}

const MAX_READERS: usize = 10;

impl FilePool {
    pub async fn open(path: &str) -> Result<Self, Error> {
        debug!("Opening {}", path);
        let writer = File::options()
            .read(false)
            .write(true)
            .append(true)
            .create(true)
            .open(path)
            .await?;

        Ok(Self {
            path: path.to_owned(),
            max_readers: MAX_READERS,
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    pub async fn reader(&mut self) -> Result<File, Error> {
        let file = File::options()
            .read(true)
            .write(false)
            .append(false)
            .open(&self.path)
            .await?;

        Ok(file)
    }

    pub async fn writer(&mut self) -> Result<OwnedMutexGuard<File>, Error> {
        let file = self.writer.clone().lock_owned().await;

        Ok(file)
    }
}
