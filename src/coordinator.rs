use std::collections::HashMap;
use std::net::Ipv4Addr;

use actix::prelude::*;
use bitvec::prelude::*;
use log::{debug, error, warn};
use rand::RngCore;

use crate::filer::{Filer, PieceRequest};
use crate::metadata::Info;
use crate::peer::{AssignPieces, BittorrentMessage, Handshake, NetworkError, Peer};
use crate::tracker::TrackerResponse;

pub struct Coordinator {
    filer_addr: Addr<Filer>,
    peers: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    peer_states: HashMap<(Ipv4Addr, u16), State>,
    we_have: BitVec<u8, Msb0>,
    info: Info,
    info_hash: [u8; 20],
}

impl Coordinator {
    pub fn new(info: Info, info_hash: [u8; 20], filer_addr: Addr<Filer>) -> Self {
        Self {
            peers: HashMap::new(),
            filer_addr,
            peer_states: HashMap::new(),
            we_have: BitVec::repeat(false, info.pieces.len()),
            info,
            info_hash,
        }
    }

    fn do_next(&mut self, _ctx: &mut <Self as Actor>::Context) {
        /*for state in self.peer_states.values() {
            let pieces = pieces_we_can_request(&self.we_have, &state);
        }*/
    }
}

fn pieces_we_can_request(we_have: &BitVec<u8, Msb0>, state: &State) -> Vec<u32> {
    let mut can_request_pieces = vec![];
    for i in 0..we_have.len() {
        match (
            we_have.get(i),
            state.they_have.get(i),
            state.we_requested.get(i),
        ) {
            (Some(we_have_bit_ref), Some(they_have_bit_ref), Some(we_requested_bit_ref)) => {
                if *we_have_bit_ref && *they_have_bit_ref && !we_requested_bit_ref {
                    can_request_pieces.push(i as u32);
                }
            }
            _ => error!("BitVec for state doesn't make sense"),
        }
    }

    can_request_pieces
}

impl Actor for Coordinator {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct TrackerUpdate {
    pub tracker_response: TrackerResponse,
}

impl Handler<TrackerUpdate> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: TrackerUpdate, ctx: &mut Context<Self>) -> Self::Result {
        for (ip_addr, port) in msg.tracker_response.peers.into_iter() {
            if !self.peers.contains_key(&(ip_addr, port)) {
                debug!("Setting up peer {}:{}", ip_addr, port);
                let peer = Peer::new(ip_addr, port, ctx.address(), &self.info);
                let peer_addr = peer.start();

                let mut peer_id = [0u8; 20];
                rand::rng().fill_bytes(&mut peer_id);

                peer_addr.do_send(Handshake {
                    info_hash: self.info_hash,
                    peer_id,
                });

                self.peers.insert((ip_addr, port), peer_addr);
            } else {
                debug!("Already have peer {}:{}", ip_addr, port);
            }
        }
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PeerMessage {
    pub bittorrent_message: BittorrentMessage,
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

impl Handler<PeerMessage> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: PeerMessage, ctx: &mut Context<Self>) -> Self::Result {
        let (ip_address, port) = (msg.ip_address, msg.port);

        let peer_addr = match self.peers.get(&(ip_address, port)) {
            Some(peer_addr) => peer_addr.clone(),
            None => {
                warn!("Processing peer message but looks like it was removed");
                return;
            }
        };

        if let Some(ref mut state) = self.peer_states.get_mut(&(ip_address, port)) {
            handle_state(&msg.bittorrent_message, state);

            if let BittorrentMessage::Request {
                index,
                begin,
                length,
            } = msg.bittorrent_message
                && !state.choked
            {
                self.filer_addr.do_send(PieceRequest {
                    index,
                    begin,
                    length,
                    ip_address,
                    port,
                    peer_addr,
                });
            }
        }

        self.do_next(ctx);
    }
}

struct State {
    they_have: BitVec<u8, Msb0>,
    we_requested: BitVec<u8, Msb0>,
    interested: bool,
    choked: bool,
    we_choking: bool,
    we_interested: bool,
    assigned_pieces: BitVec,
}

impl State {
    fn new(info: &Info) -> Self {
        Self {
            they_have: BitVec::repeat(false, info.pieces.len() as usize),
            we_requested: BitVec::repeat(false, info.pieces.len() as usize),
            interested: false,
            choked: false,
            assigned_pieces: BitVec::repeat(false, info.pieces.len()),
            we_choking: false,
            we_interested: false,
        }
    }
}

struct RequestedPiece {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

fn handle_state(msg: &BittorrentMessage, state: &mut State) {
    match msg {
        BittorrentMessage::KeepAlive => {
            // Do nothing
        }
        BittorrentMessage::Choke => {
            state.choked = true;
        }
        BittorrentMessage::Unchoke => {
            state.choked = true;
        }
        BittorrentMessage::Interested => {
            state.interested = true;
        }
        BittorrentMessage::NotInterested => {
            state.interested = false;
        }
        BittorrentMessage::Have { piece_index } => {
            state.they_have.set(*piece_index as usize, true);
        }
        BittorrentMessage::BitField { bitfield } => {
            state.they_have = BitVec::<_, Msb0>::from_slice(&bitfield);
        }
        _ => (),
    };
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RecvPiece {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct HandshakeEnd {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub error: Option<NetworkError>,
}

impl Handler<HandshakeEnd> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: HandshakeEnd, _ctx: &mut <Self as Actor>::Context) -> Self::Result {
        let peer_addr = match self.peers.get(&(msg.ip_address, msg.port)) {
            Some(p) => p.clone(),
            None => {
                warn!(
                    "Handshake end but no record of peer {}:{}",
                    msg.ip_address, msg.port
                );
                return;
            }
        };

        if msg.error.is_some() {
            error!(
                "Removing peer {}:{} due to handshake error",
                msg.ip_address, msg.port
            );
            self.peers.remove(&(msg.ip_address, msg.port));
        }

        let assigned_pieces = BitVec::repeat(true, self.info.pieces.len());
        peer_addr.do_send(AssignPieces { assigned_pieces });
    }
}
