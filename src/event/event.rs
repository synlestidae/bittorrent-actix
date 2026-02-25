use actix::prelude::*;
use std::net::Ipv4Addr;

use crate::bittorrent::BittorrentMessage;
use crate::bittorrent::Info;
use crate::peer::Peer;

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub enum ConnectionEvent {
    Connected {
        ip_address: Ipv4Addr,
        port: u16,
        peer: Addr<Peer>,
    },
    Disconnected {
        ip_address: Ipv4Addr,
        port: u16,
    },
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct RecvMessageEvent {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub messages: Vec<BittorrentMessage>,
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct TrackerUpdateEvent {
    pub peers: Vec<(Ipv4Addr, u16)>,
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub struct MetadataEvent {
    pub info: Info,
}

#[derive(Message, Clone)]
#[rtype(result = "()")]
pub enum Registration {
    Connection {
        recipient: Recipient<ConnectionEvent>,
    },
    RecvMessage {
        ip_address: Ipv4Addr,
        port: u16,
        recipient: Recipient<RecvMessageEvent>,
    },
    Metadata {
        recipient: Recipient<MetadataEvent>,
    },
}
