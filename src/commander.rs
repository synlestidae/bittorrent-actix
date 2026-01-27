use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use bitvec::{order::Msb0, vec::BitVec};
use std::cmp::min;

use crate::{
    bittorrent::BittorrentMessage,
    event::{Event, EventHandler, RegId, Registration},
    piece_downloader::PieceDownloader,
    piece_uploader::PieceUploader,
};

use log::error;

const THEY_HAVE_MAX_BITS: usize = 800_000_000;

struct PeerState {
    pub handshake_sent: bool,
    pub handshake_received: bool,
    pub bitfield_sent: bool,
    pub them_choking: bool,
    pub them_interested: bool,
    pub us_choking: bool,
    pub us_interested: bool,
    pub they_have: BitVec<u8, Msb0>,
}

impl PeerState {
    fn new() -> Self {
        Self {
            handshake_sent: false,
            handshake_received: false,
            bitfield_sent: false,
            them_choking: false,
            them_interested: false,
            us_choking: false,
            us_interested: false,
            they_have: BitVec::new(),
        }
    }

    fn set_have(&mut self, piece_index: u32) -> bool {
        if piece_index as usize >= THEY_HAVE_MAX_BITS {
            return false;
        }
        let new_size = min(THEY_HAVE_MAX_BITS, piece_index as usize);
        self.they_have.resize(new_size, false);

        if let Some(ref mut bit_ref) = self.they_have.get_mut(piece_index as usize) {
            bit_ref.set(true);
            true
        } else {
            false
        }
    }
}

pub struct Commander {
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    piece_uploaders: HashMap<u32, Addr<PieceUploader>>,
    piece_downloaders: HashMap<u32, Addr<PieceDownloader>>,
    event_handler: Addr<EventHandler>,
}

impl Commander {
    pub fn new(event_handler: Addr<EventHandler>) -> Self {
        Self {
            peers: HashMap::new(),
            piece_uploaders: HashMap::new(),
            piece_downloaders: HashMap::new(),
            event_handler,
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        let fut = self.event_handler.send(Registration::Activate {
            id: RegId::next(),
            predicate: None,
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to register commander");
        }));
    }
}

impl Actor for Commander {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}

impl Handler<Event> for Commander {
    type Result = ();

    fn handle(&mut self, msg: Event, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            Event::PeerConnect { ip_address, port } => {
                self.peers.insert((ip_address, port), PeerState::new());
            }
            Event::PeerRecvMessages {
                ip_address,
                port,
                messages,
            } => {
                if let Some(peer) = self.peers.get_mut(&(ip_address, port)) {
                    handle_incoming_messages(peer, messages);
                }
            }
            Event::PeerDeath { ip_address, port } => {
                self.peers.remove(&(ip_address, port));
            }
            _ => {}
        }
    }
}

fn handle_incoming_messages(peer_state: &mut PeerState, messages: Vec<BittorrentMessage>) {
    for msg in messages.into_iter() {
        match msg {
            BittorrentMessage::Choke => peer_state.them_choking = true,
            BittorrentMessage::Unchoke => peer_state.them_choking = false,
            BittorrentMessage::Interested => peer_state.them_interested = true,
            BittorrentMessage::NotInterested => peer_state.them_interested = false,
            BittorrentMessage::Have { piece_index } => {
                let piece_set = peer_state.set_have(piece_index);
                if !piece_set {
                    error!("Peer exceeded maximum piece index");
                }
            }
            BittorrentMessage::Bitfield { bitfield } => {
                peer_state.they_have = BitVec::from_vec(bitfield);
            }
            BittorrentMessage::KeepAlive => (),
            BittorrentMessage::Cancel { .. } => (),
            BittorrentMessage::Port { .. } => (),
            BittorrentMessage::Handshake { .. } => {
                peer_state.handshake_received = true;
            }
            _ => {}
        }
    }
}
