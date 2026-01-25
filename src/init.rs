use actix::prelude::*;

use crate::coordinator::Coordinator;
use crate::filer::Filer;
use crate::filer::PieceFile;
use crate::tracker::MetadataSource;
use crate::tracker::TrackerTracker;

pub struct Init {
    tracker_tracker: Addr<TrackerTracker>,
}

impl Init {
    pub fn new(metadata_source: MetadataSource, piece_file: PieceFile) -> Self {
        let (metadata, info_hash) = match metadata_source.clone() {
            MetadataSource::Magnet { .. } => todo!("magnet not yet implemented"),
            MetadataSource::Metadata {
                info_hash,
                metadata,
            } => (metadata, info_hash),
        };

        let filer_addr = Filer::new(metadata, piece_file).start();
        let coordinator_addr = Coordinator::new(info_hash, filer_addr).start();
        let tracker_tracker = TrackerTracker::new(metadata_source, coordinator_addr.recipient());

        Self {
            tracker_tracker: tracker_tracker.start(),
        }
    }
}

impl Actor for Init {
    type Context = Context<Self>;
}

/*
#[derive(Message, Debug)]
#[rtype(result = "Result<(), InitError>")]
pub struct InitDownload {}
*/

/*impl Handler<InitDownload> for Init {
    type Result = ResponseActFuture<Self, Result<(), InitError>>;

    fn handle(&mut self, _msg: InitDownload, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received InitDownload");
        let metadata = self.metadata.clone();
        let tracker_addr = self.tracker_addr.clone();
        let piece_file = self.piece_file.clone();

        Box::pin(
            async move {
                let tracker_response = match tracker_addr.send(AskTracker {}).await {
                    Ok(result) => result?,
                    // TODO handle when we repeat InitDownload
                    Err(_) => unreachable!("Should not fail to send message on startup"),
                };

                coordinator_addr
                    .send(TrackerUpdate {
                        tracker_response: Some(tracker_response),
                        peers: Vec::new(),
                    })
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
}*/
/*
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
}*/
