use crate::event::Event;
use crate::event::EventHandler;
use crate::event::RegId;
use crate::event::Registration;
use crate::peer::Peer;

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use log::debug;
use rand::Rng;

use std::time::Instant;

pub struct PeerSwarm {
    peers: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    event_handler: Addr<EventHandler>,
    moratorium: HashMap<(Ipv4Addr, u16), Instant>,
}

impl PeerSwarm {
    pub fn new(event_handler: Addr<EventHandler>) -> Self {
        Self {
            peers: HashMap::new(),
            event_handler,
            moratorium: HashMap::new(),
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        debug!("Registering peer swarm");

        let fut = self.event_handler.send(Registration::Activate {
            id: RegId::next(),
            predicate: None,
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

const MAX_PEERS: usize = 100;

impl Handler<Event> for PeerSwarm {
    type Result = ();

    fn handle(&mut self, msg: Event, _ctx: &mut Self::Context) -> Self::Result {
        self.clear_moratorium();

        let now = Instant::now();
        let mut random = rand::rng();

        self.peers.retain(|(ip_address, port), addr| {
            if !addr.connected() {
                let fuzz = random.random_range(0..30);
                let instant = now + Duration::from_secs(fuzz);

                self.moratorium.insert(
                    (*ip_address, *port),
                    instant,
                );

                return false;
            }

            true
        });

        let mut peers = match msg {
            Event::PeerList { peers } => peers,
            Event::PeerDeath { ip_address, port } => {
                let fuzz = random.random_range(0..30);
                let instant = now + Duration::from_secs(fuzz);

                self.moratorium.insert(
                    (ip_address, port),
                    instant,
                );

                return;
            },
            _ => return,
        };

        peers.sort_by_key(|(ip_address, port)| {
            match self.moratorium.get(&(*ip_address, *port)) {
                Some(moratorium_timestamp) => Duration::from_secs(u64::max_value()) - now.duration_since(*moratorium_timestamp),
                None => return Duration::from_secs(0)
            }
        });

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
