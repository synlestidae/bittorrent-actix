use std::process::exit;

use actix::prelude::*;
use log::{debug, error};

use crate::bittorrent::{Magnet, Metadata, PeerId};
use crate::commander::Commander;
use crate::event::EventHandler;
use crate::metadata_coordinator::MetadataCoordinator;
use crate::peer_swarm::PeerSwarm;
use crate::tracker::TrackerTracker;
use rand::Rng;
use rand::rng;

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
        let mut peer_id = [0u8; 20];
        rng().fill(&mut peer_id);

        let tracker_tracker = match &options {
            InitOptions::Metadata(metadata) => {
                TrackerTracker::with_metadata(info_hash, metadata.clone(), event_handler.clone())
            }
            InitOptions::Magnet(magnet) => {
                TrackerTracker::with_magnet(magnet.clone(), event_handler.clone())
            }
        };

        let peer_swarm = PeerSwarm::new(event_handler.clone());

        let commander = Commander::new(info_hash, PeerId(peer_id), event_handler.clone());

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

    fn started(&mut self, _ctx: &mut Self::Context) {
        debug!("Init started"); 
    }


    fn stopped(&mut self, ctx: &mut Self::Context) {
        error!("Init stopped");
        exit(1);
    }
}
