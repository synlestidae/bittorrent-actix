use std::{cmp::min, collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use bitvec::{order::Msb0, vec::BitVec};

use crate::{
    coordinator::{self, Coordinator},
    metadata::{Info, PieceRequest},
    peer::{PeerControl, PeerControlEnvelope},
    peer_state::PeerState,
};

pub struct Commander {
    coordinator_addr: Addr<Coordinator>,
    we_have: BitVec<u8, Msb0>,
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    info: Info,
}

impl Commander {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>) -> Commander {
        Commander {
            coordinator_addr,
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
                let assigned_pieces: Vec<u32> = (!self.we_have.clone()
                    & peer_state.they_have.clone())
                .iter_ones()
                .map(|i| i as u32)
                .collect();
                peer_state.assign_pieces(assigned_pieces.iter().cloned().collect());
                messages.push(PeerControl::AssignPieces { assigned_pieces });
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
    PieceCompleted {
        piece_index: u32,
    },
    PieceRequest {
        index: u32,
        begin: u32,
        length: u32,
    },
}

#[derive(Message)]
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
                self.peers
                    .insert((envelope.ip_address, envelope.port), PeerState::new());
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
                        peer.them_choking = them_choking;
                    }
                    if let Some(them_interested) = interested {
                        peer.them_interested = them_interested;
                    }
                    if let Some(bitfield) = bitfield {
                        peer.they_have = peer.they_have.clone() | bitfield;
                    }
                    if let Some(piece_index) = have {
                        let new_size = min(self.info.pieces.len(), piece_index as usize);
                        peer.they_have.resize(new_size, false);
                        peer.they_have.set(piece_index as usize, true);
                    }
                }
                (CommanderMsg::PieceCompleted { piece_index }, Some(peer)) => {
                    peer.pieces_received.insert(piece_index);
                }
                (
                    CommanderMsg::PieceRequest {
                        index,
                        begin,
                        length,
                    },
                    Some(peer),
                ) => {
                    peer.piece_requests.push(PieceRequest {
                        index,
                        begin,
                        length,
                    });
                }
                _ => (),
            }
        }

        let command_set = self.command_set();
        self.coordinator_addr.do_send(command_set);
    }
}
