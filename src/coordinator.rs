use std::collections::HashMap;
use std::net::Ipv4Addr;

use actix::prelude::*;
use bitvec::prelude::*;
use log::{debug, error, warn};
use rand::RngCore;
use tokio::task::JoinSet;

use crate::filer::{Filer, PieceRequest};
use crate::metadata::Info;
use crate::peer::{BittorrentMessage, Handshake, NetworkError, Peer};
use crate::peer_state::PeerState;
use crate::tracker::TrackerResponse;

pub struct Coordinator {
    filer_addr: Addr<Filer>,
    peers: HashMap<(Ipv4Addr, u16), (Addr<Peer>, PeerState)>,
    we_have: BitVec<u8, Msb0>,
    info: Info,
    info_hash: [u8; 20],
}

impl Coordinator {
    pub fn new(info: Info, info_hash: [u8; 20], filer_addr: Addr<Filer>) -> Self {
        Self {
            filer_addr,
            peers: HashMap::new(),
            we_have: BitVec::repeat(false, info.pieces.len()),
            info,
            info_hash,
        }
    }
}

impl Actor for Coordinator {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<(), NetworkError>")]
pub struct TrackerUpdate {
    pub tracker_response: TrackerResponse,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PeerMessage {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub msg: BittorrentMessage,
}

impl Handler<TrackerUpdate> for Coordinator {
    type Result = ResponseActFuture<Self, Result<(), NetworkError>>;

    fn handle(&mut self, msg: TrackerUpdate, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received TrackerUpdate");
        let new_peers: Vec<_> = msg
            .tracker_response
            .peers
            .into_iter()
            .filter(|(ip_address, port)| !self.peers.contains_key(&(*ip_address, *port)))
            .collect();

        let info = self.info.clone();
        let info_for_peer = self.info.clone();
        let info_hash = self.info_hash.clone();
        let address = ctx.address();

        debug!("Tracker gave us {} new peers", new_peers.len());

        return Box::pin(
            async move {
                let mut joinset = JoinSet::new();
                let mut peers = Vec::new();
                for (ip_address, port) in new_peers {
                    debug!("Setting up peer {}:{}", ip_address, port);

                    let peer = Peer::new(ip_address, port, address.clone(), &info);
                    let peer_addr = peer.start();

                    let mut peer_id = [0u8; 20];
                    rand::rng().fill_bytes(&mut peer_id);

                    joinset.spawn(peer_addr.send(Handshake {
                        info_hash: info_hash,
                        peer_id,
                    }));

                    peers.push((ip_address, port, peer_addr));
                }

                let mut new_peers = Vec::new();
                for (i, join_result) in joinset.join_all().await.into_iter().enumerate() {
                    let (ip_address, port, peer_addr) = &peers[i];
                    if let Ok(_) = join_result {
                        debug!("Successful handshake with peer {}:{}", ip_address, port);
                        new_peers.push((*ip_address, *port, peer_addr.clone()));
                    } else {
                        debug!("Dropping peer {}:{}", ip_address, port);
                    }
                }

                new_peers
            }
            .into_actor(self)
            .map(move |new_peers, actor, _ctx| {
                for (ip_address, port, peer_addr) in new_peers.into_iter() {
                    actor.peers.insert(
                        (ip_address, port),
                        (peer_addr, PeerState::new(&info_for_peer)),
                    );
                }
                Ok(())
            }),
        );
    }
}

impl Handler<PeerMessage> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: PeerMessage, ctx: &mut Context<Self>) -> Self::Result {
        let (ip_address, port) = (msg.ip_address, msg.port);

        let (_, peer_state) = match self.peers.get_mut(&(ip_address, port)) {
            Some((peer_addr, peer_state)) => (peer_addr.clone(), peer_state),
            None => {
                warn!("Peer {}:{} not found", ip_address, port);
                return;
            }
        };

        peer_state.new_message(&msg.msg);

        if let Some((peer_addr, state)) = self.peers.get(&(ip_address, port)) {
            if let BittorrentMessage::Request {
                index,
                begin,
                length,
            } = msg.msg
                && !state.are_choking_them()
            {
                self.filer_addr.do_send(PieceRequest {
                    index,
                    begin,
                    length,
                    ip_address,
                    port,
                    peer_addr: peer_addr.clone(),
                });
            }
        }
    }
}

struct RequestedPiece {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}
