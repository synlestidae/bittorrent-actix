use actix::prelude::*;

use crate::commander::Commander;
use crate::event::EventHandler;
use crate::metadata_coordinator::MetadataCoordinator;
use crate::peer_swarm::PeerSwarm;
use crate::tracker::MetadataSource;
use crate::tracker::TrackerTracker;

pub struct Init {
    tracker_tracker: Addr<TrackerTracker>,
    peer_swarm: Addr<PeerSwarm>,
    commander: Addr<Commander>,
    metadata_coordinator: Addr<MetadataCoordinator>,
}

impl Init {
    pub fn new(metadata_source: MetadataSource, event_handler: Addr<EventHandler>) -> Self {
        let tracker_tracker =
            TrackerTracker::new(metadata_source.clone(), event_handler.clone().recipient());

        let peer_swarm = PeerSwarm::new(event_handler.clone());

        let commander = Commander::new(event_handler.clone());

        let metadata_coordinator = MetadataCoordinator::new(metadata_source, event_handler.clone());

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
