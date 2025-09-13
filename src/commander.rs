use std::{
    cmp::{max},
    collections::HashMap,
    net::Ipv4Addr,
};

use actix::prelude::*;
use bitvec::{order::Msb0, vec::BitVec};
use log::debug;

use crate::{
    coordinator::Coordinator, filer::{Filer, WholePiece}, metadata::{Info, Piece, PieceRequest}, peer::{PeerControl, PeerControlEnvelope}, peer_state::PeerState
};

pub struct Commander {
    coordinator_addr: Addr<Coordinator>,
    filer_addr: Addr<Filer>,
    we_have: BitVec<u8, Msb0>,
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    info: Info,
}

const MAX_PIECE_REQUESTS: usize = 10;

impl Commander {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>, filer_addr: Addr<Filer>) -> Commander {
        Commander {
            coordinator_addr,
            filer_addr,
            we_have: BitVec::new(),
            peers: HashMap::new(),
            info,
        }
    }

    pub fn command_set(&mut self) -> CommandSet {
        let mut peer_control = Vec::new();

        for ((ip_address, port), peer_state) in self.peers.iter_mut() {
            let mut messages = Vec::new();

            if peer_state.us_choking {
                messages.push(PeerControl::UnchokeThem);
                peer_state.us_choking = false;
            }

            if !peer_state.us_interested {
                messages.push(PeerControl::Interested);
                peer_state.us_interested = true;
            }

            if peer_state.assigned_pieces.len() == 0 {
                let len = self.info.num_pieces();
                self.we_have.resize(len, false);
                peer_state.they_have.resize(len, false);

                let assigned_pieces: Vec<u32> =
                    peer_state.they_have.iter_ones().map(|i| i as u32).collect();

                if assigned_pieces.len() == 0 {
                    debug!("No pieces to assign");
                } else {
                    peer_state.assign_pieces(assigned_pieces.iter().cloned().collect());
                    messages.push(PeerControl::AssignPieces { assigned_pieces });
                }
            }

            // complete any pieces
            let complete_pieces = peer_state.piece_tracker.complete_pieces();
            debug!("{} complete piece(s)", complete_pieces.len());
            for complete_piece in complete_pieces {
                debug!("Piece {} is complete", complete_piece.index);
                peer_state.piece_tracker.mark_complete(complete_piece.index);
                peer_state.piece_tracker.remove(complete_piece.index);

                if self
                    .info
                    .piece_correct(complete_piece.index, &complete_piece.data)
                {
                    debug!("Piece {} is correct", complete_piece.index);

                    self.filer_addr.do_send(WholePiece {
                        index: complete_piece.index,
                        piece: complete_piece.data,
                        ip_address: *ip_address,
                        port: *port,
                    });

                    // TODO we dump the piece, but need to send it to filer
                } else {
                    debug!("Piece {} is incorrect", complete_piece.index);
                }
            }

            // get next piece requests
            let piece_requests = peer_state
                .piece_tracker
                .next_pieces(&peer_state.assigned_pieces, MAX_PIECE_REQUESTS);
            if piece_requests.len() > 0 {
                messages.push(PeerControl::RequestPieces { piece_requests });
            }

            peer_control.push(PeerControlEnvelope {
                ip_address: *ip_address,
                port: *port,
                messages,
            });
        }

        CommandSet { peer_control }
    }
}

impl Actor for Commander {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct CommanderMsgEnvelope {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub msgs: Vec<CommanderMsg>,
}

pub enum CommanderMsg {
    AddPeer,
    RemovePeer,
    PeerChange {
        choking: Option<bool>,
        interested: Option<bool>,
        bitfield: Option<BitVec<u8, Msb0>>,
        have: Option<u32>,
    },
    PieceRequest {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct CommandSet {
    pub peer_control: Vec<PeerControlEnvelope>,
}

impl Handler<CommanderMsgEnvelope> for Commander {
    type Result = ();

    fn handle(&mut self, envelope: CommanderMsgEnvelope, _ctx: &mut Self::Context) -> Self::Result {
        for msg in envelope.msgs.into_iter() {
            if let &CommanderMsg::RemovePeer = &msg {
                self.peers.remove(&(envelope.ip_address, envelope.port));
            } else if !self
                .peers
                .contains_key(&(envelope.ip_address, envelope.port))
            {
                self.peers.insert(
                    (envelope.ip_address, envelope.port),
                    PeerState::new(self.info.clone()),
                );
            }

            match (
                msg,
                self.peers.get_mut(&(envelope.ip_address, envelope.port)),
            ) {
                (
                    CommanderMsg::PeerChange {
                        choking,
                        interested,
                        bitfield,
                        have,
                    },
                    Some(peer),
                ) => {
                    if let Some(them_choking) = choking {
                        debug!(
                            "Peer {}:{} set choking to {}",
                            envelope.ip_address, envelope.port, them_choking
                        );
                        peer.them_choking = them_choking;
                    }
                    if let Some(them_interested) = interested {
                        debug!(
                            "Peer {}:{} set interested to {}",
                            envelope.ip_address, envelope.port, them_interested
                        );

                        peer.them_interested = them_interested;
                    }
                    if let Some(bitfield) = bitfield {
                        debug!(
                            "Peer {}:{} set bitfield to {}",
                            envelope.ip_address, envelope.port, bitfield
                        );

                        peer.they_have = peer.they_have.clone() | bitfield;
                    }
                    if let Some(piece_index) = have {
                        debug!(
                            "Peer {}:{} has piece {}",
                            envelope.ip_address, envelope.port, piece_index
                        );

                        let new_size = max(self.info.num_pieces(), piece_index as usize);
                        peer.they_have.resize(new_size, false);
                        peer.they_have.set(piece_index as usize, true);
                    }
                }
                (
                    CommanderMsg::PieceRequest {
                        index,
                        begin,
                        length,
                    },
                    Some(peer),
                ) => {
                    debug!(
                        "Requesting piece index={}, begin={}, length={} from peer {}:{}",
                        index, begin, length, envelope.ip_address, envelope.port
                    );
                    peer.piece_requests.push(PieceRequest {
                        index,
                        begin,
                        length,
                    });
                }
                (
                    CommanderMsg::Piece {
                        index,
                        begin,
                        block,
                    },
                    Some(peer),
                ) => {
                    debug!(
                        "Peer {}:{} received piece Piece index = {}, begin = {}",
                        envelope.ip_address, envelope.port, index, begin
                    );
                    peer.piece_tracker.add_piece(Piece {
                        index,
                        begin,
                        block,
                    });
                }
                _ => (),
            }
        }

        let command_set = self.command_set();
        self.coordinator_addr.do_send(command_set);
    }
}
