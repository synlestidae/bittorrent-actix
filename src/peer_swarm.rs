use crate::event::ConnectionEvent;
use crate::event::EventHandler;
use crate::event::Registration;
use crate::event::TrackerUpdateEvent;
use crate::peer::Peer;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use log::debug;
use log::trace;
use rand::Rng;

use std::time::Instant;

pub struct PeerSwarm {
    available_peers: HashMap<(Ipv4Addr, u16), Instant>,
    peers: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    event_handler: Addr<EventHandler>,
    moratorium: HashMap<(Ipv4Addr, u16), Instant>,
}

impl PeerSwarm {
    pub fn new(event_handler: Addr<EventHandler>) -> Self {
        Self {
            available_peers: HashMap::new(),
            peers: HashMap::new(),
            event_handler,
            moratorium: HashMap::new(),
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        debug!("Registering peer swarm");

        let fut = self.event_handler.send(Registration::Connection {
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("failed to register peer swarm");
        }));
    }

    fn clear_moratorium(&mut self) {
        let now = Instant::now();
        let duration = Duration::from_secs(10 * 60);

        self.moratorium.retain(|(ip_address, port), timestamp| {
            let lifted = now.duration_since(*timestamp) > duration;
            if lifted {
                debug!("Moratorium lifted on on {}:{}", ip_address, port);
            }

            lifted
        });
    }
}

impl Actor for PeerSwarm {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}

const MAX_PEERS: usize = 50;

impl Handler<TrackerUpdateEvent> for PeerSwarm {
    type Result = ();

    fn handle(&mut self, msg: TrackerUpdateEvent, _ctx: &mut Self::Context) -> Self::Result {
        self.clear_moratorium();

        let now = Instant::now();
        let mut random = rand::rng();

        self.peers.retain(|(ip_address, port), addr| {
            if !addr.connected() {
                let fuzz = random.random_range(0..30);
                let instant = now + Duration::from_secs(fuzz);

                self.moratorium.insert((*ip_address, *port), instant);

                return false;
            }

            true
        });

        let new_peers = msg.peers;

        new_peers.into_iter().for_each(|(ip_address, port)| {
            self.available_peers.insert((ip_address, port), now);
        });

        self.available_peers
            .retain(|_, timestamp| now.duration_since(*timestamp) < Duration::from_secs(3 * 60));

        let mut peers: Vec<_> = self.available_peers.keys().cloned().collect();

        peers.sort_by_key(
            |(ip_address, port)| match self.moratorium.get(&(*ip_address, *port)) {
                Some(moratorium_timestamp) => {
                    Duration::from_secs(u64::max_value())
                        - now.duration_since(*moratorium_timestamp)
                }
                None => return Duration::from_secs(0),
            },
        );

        for (ip_address, port) in peers {
            if self.peers.len() >= MAX_PEERS {
                break;
            }
            if self.peers.contains_key(&(ip_address, port)) {
                continue;
            }

            debug!("Adding {}:{} to swarm", ip_address, port);

            let peer = Peer::new((ip_address, port), self.event_handler.clone());

            self.peers.insert((ip_address, port), peer.start());
        }

        debug!("Have {} peer(s) in swarm", self.peers.len());
    }
}

impl Handler<ConnectionEvent> for PeerSwarm {
    type Result = ();

    fn handle(&mut self, msg: ConnectionEvent, _ctx: &mut Self::Context) -> Self::Result {
        let (ip_address, port) = match msg {
            ConnectionEvent::Connected { .. } => return,
            ConnectionEvent::Disconnected { ip_address, port } => (ip_address, port),
        };
        let now = Instant::now();

        let mut random = rand::rng();
        let fuzz = random.random_range(0..30);
        let instant = now + Duration::from_secs(fuzz);

        trace!("Removing {}:{} from swarm", ip_address, port);

        self.moratorium.insert((ip_address, port), instant);
        self.peers.remove(&(ip_address, port));
    }
}
