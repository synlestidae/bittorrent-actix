use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use log::warn;

use super::peer_state::PeerState;
use crate::bittorrent::Info;
use crate::commander::Command;
use crate::commander::command::CommanderRequest;
use crate::commander::peer_state::PeerChange;

pub struct Commander {
    peers: HashMap<(Ipv4Addr, u16), PeerState>,
    info_hash: [u8; 20],
    info: Option<Info>,
}

impl Commander {
    pub fn new(info_hash: [u8; 20]) -> Commander {
        Commander {
            peers: HashMap::new(),
            info_hash,
            info: None,
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
                let peer_state = PeerState::new((ip_address, port));
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
                    return peer_state.commands(self.info_hash);
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
