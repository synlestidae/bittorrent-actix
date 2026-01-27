use actix::prelude::*;
use std::net::Ipv4Addr;

use crate::bittorrent::BittorrentMessage;
use crate::bittorrent::Metadata;
use crate::peer::Peer;
use crate::tracker::HttpTrackerResponse;

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub enum Event {
    PeerList {
        peers: Vec<(Ipv4Addr, u16)>,
    },
    Metadata {
        metadata: Metadata,
    },
    PeerConnect {
        ip_address: Ipv4Addr,
        port: u16,
    },
    PeerRecvMessages {
        ip_address: Ipv4Addr,
        port: u16,
        messages: Vec<BittorrentMessage>,
    },
    PeerDeath {
        ip_address: Ipv4Addr,
        port: u16,
    },
    /*GotMetadata {
        metadata: Metadata,
    },
    PeerConnect {
        ip_address: Ipv4Addr,
        port: u16,
        peer: Addr<Peer>,
    },
    PeerDisconnect {
        ip_address: Ipv4Addr,
        port: u16,
    },
    PeerCompletePiece {
        ip_address: Ipv4Addr,
        port: u16,
        piece_index: u32,
    },*/
}

pub const HTTP_TRACKER_UPDATE: u32 = 0b0000_0001;
pub const GOT_METADATA: u32 = 0b0000_0010;
pub const PEER_CONNECT: u32 = 0b0000_0100;
pub const PEER_DISCONNECT: u32 = 0b0000_1000;
pub const PEER_RECV_MESSAGES: u32 = 0b0001_0000;
pub const PEER_COMPLETE_PIECE: u32 = 0b0010_0000;

#[derive(Clone)]
pub struct RegId(u64);

impl RegId {
    pub fn next() -> Self {
        RegId(0) // TODO!!!
    }
}

#[derive(Clone)]
pub struct EventInfo {
    pub peer_id: Option<(Ipv4Addr, u16)>,
    pub piece_index: Option<u32>,
}

#[derive(Clone)]
pub struct EventPredicate {
    pub peer_address: Option<(Ipv4Addr, u16)>,
    pub piece_index: Option<u32>,
    pub message_type: Option<u16>,
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub enum Registration {
    Activate {
        id: RegId,
        predicate: Option<EventPredicate>,
        recipient: Recipient<Event>,
    },
}
