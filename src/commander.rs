use std::{cmp::max, collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use bitvec::{order::Msb0, vec::BitVec};

use crate::{
    coordinator::{Coordinator, PeerControlGlobal},
    filer::{Filer, RequestPiece},
    metadata::{CompletePiece, Info},
    peer::PeerControlEnvelope,
    peer_state::{PeerChange, PeerState},
};

pub struct Commander {
    coordinator_addr: Addr<Coordinator>,
    we_have: BitVec<u8, Msb0>,
    filer_addr: Addr<Filer>,
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    info: Info,
}

impl Commander {
    pub fn new(
        info: Info,
        coordinator_addr: Addr<Coordinator>,
        filer_addr: Addr<Filer>,
    ) -> Commander {
        Commander {
            coordinator_addr,
            filer_addr,
            peers: HashMap::new(),
            we_have: BitVec::repeat(false, info.num_pieces()),
            info,
        }
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
    SetPieceAssignment { piece_indexes: Vec<u32> },
    PeerChange { peer_change: PeerChange },
}

#[derive(Debug)]
pub struct PeerChangeEnvelope {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub peer_change: PeerChange,
}

#[derive(Message, Debug, Default)]
#[rtype(result = "()")]
pub struct CommandSet {
    pub peer_change: Vec<PeerChangeEnvelope>,
    pub peer_control: Vec<PeerControlEnvelope>,
    pub peer_control_global: Vec<PeerControlGlobal>,
    pub complete_pieces: Vec<CompletePiece>,
    pub piece_requests_to_us: Vec<RequestPiece>,
}

impl Handler<CommanderMsgEnvelope> for Commander {
    type Result = ();

    fn handle(&mut self, envelope: CommanderMsgEnvelope, ctx: &mut Self::Context) -> Self::Result {
        for msg in envelope.msgs.iter() {
            match (
                msg,
                self.peers
                    .contains_key(&(envelope.ip_address, envelope.port)),
            ) {
                (CommanderMsg::AddPeer, false) => {
                    let address = (envelope.ip_address, envelope.port);
                    let info = self.info.clone();
                    let we_have = self.we_have.clone();
                    let peer_state = PeerState::new(info, address, we_have);
                    self.peers
                        .insert((envelope.ip_address, envelope.port), peer_state);
                }
                (CommanderMsg::RemovePeer, true) => {
                    self.peers.remove(&(envelope.ip_address, envelope.port));
                }
                _ => (),
            };
        }

        ctx.set_mailbox_capacity(max(self.peers.len() * 5, 5));

        let peer_state = match self.peers.get_mut(&(envelope.ip_address, envelope.port)) {
            Some(peer_state) => peer_state,
            None => return,
        };

        for msg in envelope.msgs.into_iter() {
            match msg {
                CommanderMsg::PeerChange { peer_change } => {
                    peer_state.change(peer_change);
                }
                CommanderMsg::SetPieceAssignment { piece_indexes } => {
                    peer_state.set_piece_assignment(piece_indexes);
                }
                _ => (),
            }
        }

        let peer_command = peer_state.peer_command();
        self.coordinator_addr.do_send(CommandSet {
            peer_control: vec![PeerControlEnvelope {
                ip_address: peer_state.ip_address,
                port: peer_state.port,
                messages: peer_command.peer_control,
            }],
            peer_control_global: peer_command.peer_control_global,
            complete_pieces: peer_command.complete_pieces,
            piece_requests_to_us: peer_command.piece_requests_to_us,
            peer_change: vec![],
        });
    }
}
