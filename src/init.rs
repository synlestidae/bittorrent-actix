use actix::prelude::*;

use crate::coordinator::Coordinator;
use crate::filer::Filer;
use crate::metadata::Metadata;
use crate::tracker::{AskTracker, Tracker};

use log::debug;

pub struct Init {
    tracker: Addr<Tracker>,
    coordinator: Addr<Coordinator>,
    filer: Addr<Filer>,
}

impl Default for Init {
    fn default() -> Self {
        let coordinator: Coordinator = Default::default();
        let tracker: Tracker = Default::default();
        let filer: Filer = Default::default();

        Self {
            tracker: tracker.start(),
            coordinator: coordinator.start(),
            filer: filer.start(),
        }
    }
}

impl Actor for Init {
    type Context = Context<Self>;
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct InitDownload {
    pub metadata: Metadata,
    pub info_hash: Vec<u8>,
    pub left: u64,
}

impl Handler<InitDownload> for Init {
    type Result = ();

    fn handle(&mut self, msg: InitDownload, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received InitDownload");

        self.tracker.do_send(AskTracker {
            announce_url: msg.metadata.announce,
            info_hash: msg.info_hash,
            left: msg.left,
            coordinator: self.coordinator.clone(),
        });
    }
}
