use actix::prelude::*;

use crate::coordinator::{Coordinator, TrackerUpdate};
use crate::file::FilePool;
use crate::filer::Filer;
use crate::metadata::Metadata;
use crate::tracker::{AskTracker, Tracker, TrackerError};
use tokio::io;

use log::debug;

pub struct Init {
    metadata: Metadata,
    file_pool: FilePool,
    tracker_addr: Addr<Tracker>,
    coordinator_addr: Option<Addr<Coordinator>>,
    filer_addr: Option<Addr<Filer>>,
    info_hash: [u8; 20],
}

impl Init {
    pub async fn new(metadata: Metadata, file_pool: FilePool, info_hash: [u8; 20]) -> Self {
        let tracker_addr = Tracker::new(metadata.clone(), info_hash).start();

        Self {
            metadata,
            tracker_addr,
            coordinator_addr: None,
            filer_addr: None,
            file_pool,
            info_hash,
        }
    }
}

impl Actor for Init {
    type Context = Context<Self>;
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), InitError>")]
pub struct InitDownload {}

impl Handler<InitDownload> for Init {
    type Result = ResponseActFuture<Self, Result<(), InitError>>;

    fn handle(&mut self, _msg: InitDownload, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received InitDownload");
        let info_hash = self.info_hash.clone();
        let info = self.metadata.info.clone();
        let tracker_addr = self.tracker_addr.clone();

        Box::pin(
            async move {
                let tracker_response = match tracker_addr.send(AskTracker {}).await {
                    Ok(result) => result?,
                    // TODO handle when we repeat InitDownload
                    Err(_) => unreachable!("Should not fail to send message on startup"),
                };

                let filer_addr = Filer::new(info.clone()).await?.start();
                let coordinator_addr = Coordinator::create(|ctx| {
                    Coordinator::new(info, info_hash, filer_addr.clone(), ctx)
                });

                coordinator_addr
                    .send(TrackerUpdate { tracker_response })
                    .await;

                Ok((filer_addr, coordinator_addr))
            }
            .into_actor(self)
            .map(|result: Result<_, InitError>, actor, _| {
                let (filer_addr, coordinator_addr) = result?;
                actor.filer_addr = Some(filer_addr);
                actor.coordinator_addr = Some(coordinator_addr);

                Ok(())
            }),
        )
    }
}

pub enum InitError {
    Io(io::Error),
    Mailbox(MailboxError),
    Tracker(TrackerError),
}

impl From<io::Error> for InitError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<MailboxError> for InitError {
    fn from(err: MailboxError) -> Self {
        Self::Mailbox(err)
    }
}

impl From<TrackerError> for InitError {
    fn from(err: TrackerError) -> Self {
        Self::Tracker(err)
    }
}
