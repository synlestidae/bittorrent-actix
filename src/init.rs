use actix::prelude::*;

use crate::coordinator::Coordinator;
use crate::filer::Filer;
use crate::metadata::Metadata;
use crate::tracker::{AskTracker, Tracker};

use log::debug;

pub struct Init {
    tracker_addr: Addr<Tracker>,
}

impl Init {
    pub fn new(metadata: Metadata, info_hash: [u8; 20]) -> Self {
        let filer_addr = Filer::new().start();
        let coordinator_addr =
            Coordinator::new(metadata.info.clone(), info_hash.clone(), filer_addr.clone()).start();
        let tracker_addr =
            Tracker::new(metadata, coordinator_addr, filer_addr.clone(), info_hash).start();

        Self { tracker_addr }
    }
}

impl Actor for Init {
    type Context = Context<Self>;
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct InitDownload {}

impl Handler<InitDownload> for Init {
    type Result = ();

    fn handle(&mut self, _msg: InitDownload, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received InitDownload");

        self.tracker_addr.do_send(AskTracker {});
    }
}
