use std::{collections::HashMap, net::Ipv4Addr};

use crate::{
    bittorrent::{BittorrentHandshake, PeerId},
    event::{ConnectionEvent, RecvMessageEvent},
    peer::{Peer, SendMessages},
};
use actix::prelude::*;
use bitvec::{order::Msb0, vec::BitVec};
use log::{debug, info, trace};
use std::cmp::min;

use crate::{
    bittorrent::BittorrentMessage,
    event::{EventHandler, Registration},
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
    info_hash: [u8; 20],
    peer_id: PeerId,
    peers: HashMap<(Ipv4Addr, u16), (PeerState, Addr<Peer>)>,
    piece_uploaders: HashMap<u32, Addr<PieceUploader>>,
    piece_downloaders: HashMap<u32, Addr<PieceDownloader>>,
    event_handler: Addr<EventHandler>,
}

impl Commander {
    pub fn new(info_hash: [u8; 20], peer_id: PeerId, event_handler: Addr<EventHandler>) -> Self {
        Self {
            info_hash,
            peer_id,
            peers: HashMap::new(),
            piece_uploaders: HashMap::new(),
            piece_downloaders: HashMap::new(),
            event_handler,
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        debug!("Registering commander");
        let fut = self.event_handler.send(Registration::Connection {
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to register commander");
        }));
    }

    fn send_messages(&mut self, ctx: &mut Context<Self>) {
        // do the handshakes
        trace!("Can command {} peer(s)", self.peers.len());

        let addresses: Vec<_> = self
            .peers
            .iter()
            .filter_map(|((ip_address, port), (peer_state, addr))| {
                if !peer_state.handshake_sent {
                    Some((*ip_address, *port, addr))
                } else {
                    None
                }
            })
            .collect();

        debug!("Got {} handshake(s) to send", addresses.len());

        for (ip_address, port, addr) in addresses.iter() {
            let handshake = BittorrentHandshake::new(self.peer_id, self.info_hash);

            let fut = addr.send(SendMessages {
                messages: vec![
                    BittorrentMessage::Handshake { handshake },
                    BittorrentMessage::Unchoke,
                ],
            });
            trace!("Sending handshake to {}:{}", ip_address, port);

            let (ip_address, port) = (ip_address.clone(), port.clone());

            ctx.spawn(
                fut.into_actor(self)
                    .map(move |result, actor, _| match result {
                        Err(err) => {
                            info!(
                                "Error sending handshake to {}:{}: {:?}",
                                ip_address, port, err
                            );
                            return;
                        }
                        Ok(Err(err)) => {
                            info!(
                                "Error sending handshake to {}:{}: {:?}",
                                ip_address, port, err
                            );
                            return;
                        }
                        Ok(Ok(())) => {
                            info!("Handshake sent to {}:{}", ip_address, port);

                            if let Some((peer_state, _)) = actor.peers.get_mut(&(ip_address, port))
                            {
                                peer_state.handshake_sent = true;
                                peer_state.us_choking = false;
                            }
                        }
                    }),
            ); // TODO
        }
    }
}

impl Actor for Commander {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}

impl Handler<ConnectionEvent> for Commander {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: ConnectionEvent, ctx: &mut Self::Context) -> Self::Result {
        match msg {
            ConnectionEvent::Connected {
                ip_address,
                port,
                peer,
            } => {
                self.peers
                    .insert((ip_address, port), (PeerState::new(), peer));

                return Box::pin(
                    self.event_handler
                        .send(Registration::RecvMessage {
                            ip_address,
                            port,
                            recipient: ctx.address().recipient(),
                        })
                        .into_actor(self)
                        .map(|result, _, _| {
                            result.expect("unable to register for recv message events");
                        }),
                );
            }
            ConnectionEvent::Disconnected { ip_address, port } => {
                self.peers.remove(&(ip_address, port));

                return Box::pin(async {}.into_actor(self));
            }
        }
    }
}

impl Handler<RecvMessageEvent> for Commander {
    type Result = ();

    fn handle(&mut self, msg: RecvMessageEvent, ctx: &mut Self::Context) -> Self::Result {
        todo!()
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
