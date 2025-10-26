use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use log::warn;

use super::peer_state::PeerState;
use crate::bittorrent::Info;
use crate::commander::Command;
use crate::commander::command::CommanderRequest;
use crate::commander::peer_state::PeerChange;
use crate::coordinator::Coordinator;

pub struct Commander {
    coordinator_addr: Addr<Coordinator>,
    we_have: BitVec<u8, Msb0>,
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    info: Info,
}

impl Commander {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>) -> Commander {
        let we_have = BitVec::repeat(false, info.num_pieces());
        Commander {
            coordinator_addr,
            peers: HashMap::new(),
            we_have,
            info,
        }
    }
}

impl Actor for Commander {
    type Context = Context<Self>;
}

impl Handler<CommanderRequest> for Commander {
    type Result = Vec<Command>;

    fn handle(&mut self, msg: CommanderRequest, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            CommanderRequest::AddPeer { ip_address, port } => {
                let address = (ip_address, port);
                let peer_state = PeerState::new(self.info.clone(), address, self.we_have.clone());
                self.peers.insert(address, peer_state);

                vec![]
            }
            CommanderRequest::RemovePeer { ip_address, port } => {
                self.peers.remove(&(ip_address, port));

                vec![]
            }
            CommanderRequest::RecvMessages {
                ip_address,
                port,
                messages,
            } => {
                if let Some(peer_state) = self.peers.get_mut(&(ip_address, port)) {
                    for message in messages.into_iter() {
                        peer_state.change(PeerChange::RecvMessage { message });
                    }
                    return peer_state.commands();
                }

                warn!(
                    "Processed messages but peer {}:{} is missing",
                    ip_address, port
                );

                vec![]
            }
        }
    }
}
