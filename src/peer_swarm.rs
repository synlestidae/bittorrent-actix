use crate::event::Event;
use crate::event::EventHandler;
use crate::peer::Peer;

use std::collections::HashMap;
use std::collections::HashSet;
use std::net::Ipv4Addr;

use actix::Actor;
use actix::Addr;
use actix::Context;
use actix::Handler;

pub struct PeerSwarm {
    peers: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    event_handler: Addr<EventHandler>,
}

impl PeerSwarm {
    pub fn new(event_handler: Addr<EventHandler>) -> Self {
        Self {
            peers: HashMap::new(),
            event_handler,
        }
    }
}

impl Actor for PeerSwarm {
    type Context = Context<Self>;
}

const MAX_PEERS: usize = 1024;

impl Handler<Event> for PeerSwarm {
    type Result = ();

    fn handle(&mut self, msg: Event, _ctx: &mut Self::Context) -> Self::Result {
        let peers = match msg {
            Event::PeerList { peers } => peers,
            _ => return,
        };
        let peers_already_have = self.peers.keys().cloned().collect::<HashSet<_>>();
        let response_peers = peers.iter().cloned().collect::<HashSet<_>>();

        let max = MAX_PEERS - self.peers.len(); // TODO ensure no underflow
        let new_peers_addrs = response_peers.difference(&peers_already_have).take(max);

        for (ip_address, port) in new_peers_addrs {
            let peer = Peer::new((*ip_address, *port), self.event_handler.clone());
            self.peers.insert((*ip_address, *port), peer.start());
        }
    }
}
