use std::net::Ipv4Addr;

use actix::prelude::*;

use crate::bittorrent::BittorrentMessage;
use crate::bittorrent::PieceRequest;
use crate::filer::WholePiece;

#[derive(Message)]
#[rtype(result = "Vec<Command>")]
pub enum CommanderRequest {
    AddPeer {
        ip_address: Ipv4Addr,
        port: u16,
    },
    RemovePeer {
        ip_address: Ipv4Addr,
        port: u16,
    },
    RecvMessages {
        ip_address: Ipv4Addr,
        port: u16,
        messages: Vec<BittorrentMessage>,
    },
}

pub struct CommanderResponse {
    pub commands: Vec<Command>,
}

pub enum Command {
    SendPeerMessage {
        ip_address: Ipv4Addr,
        port: u16,
        message: BittorrentMessage,
    },
    PieceRequestFromThem {
        ip_address: Ipv4Addr,
        port: u16,
        request: PieceRequest,
    },
    WholePiece {
        ip_address: Ipv4Addr,
        port: u16,
        whole_piece: WholePiece,
    },
}
