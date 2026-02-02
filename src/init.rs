use actix::prelude::*;

use crate::bittorrent::{Magnet, Metadata};
use crate::commander::Commander;
use crate::event::EventHandler;
use crate::metadata_coordinator::MetadataCoordinator;
use crate::peer_swarm::PeerSwarm;
use crate::tracker::TrackerTracker;

pub struct Init {
    tracker_tracker: Addr<TrackerTracker>,
    peer_swarm: Addr<PeerSwarm>,
    commander: Addr<Commander>,
    metadata_coordinator: Addr<MetadataCoordinator>,
}

pub enum InitOptions {
    Magnet(Magnet),
    Metadata(Metadata),
}

impl Init {
    pub fn with_metadata(
        info_hash: [u8; 20],
        options: InitOptions,
        event_handler: Addr<EventHandler>,
    ) -> Self {
        let tracker_tracker = match &options {
            InitOptions::Metadata(metadata) => TrackerTracker::with_metadata(
                info_hash,
                metadata.clone(),
                event_handler.clone().recipient(),
            ),
            InitOptions::Magnet(magnet) => TrackerTracker::with_magnet(
                info_hash,
                magnet.clone(),
                event_handler.clone().recipient(),
            ),
        };

        let peer_swarm = PeerSwarm::new(event_handler.clone());

        let commander = Commander::new(event_handler.clone());

        let metadata_coordinator = match options {
            InitOptions::Metadata(metadata) => {
                MetadataCoordinator::new(Some(metadata), None, event_handler)
            }
            InitOptions::Magnet(magnet) => {
                MetadataCoordinator::new(None, Some(magnet), event_handler)
            }
        };

        Self {
            tracker_tracker: tracker_tracker.start(),
            peer_swarm: peer_swarm.start(),
            commander: commander.start(),
            metadata_coordinator: metadata_coordinator.start(),
        }
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Ping {}

impl Handler<Ping> for Init {
    type Result = ();

    fn handle(&mut self, _msg: Ping, _ctx: &mut Self::Context) -> Self::Result {
        return ();
    }
}

impl Actor for Init {
    type Context = Context<Self>;
}
