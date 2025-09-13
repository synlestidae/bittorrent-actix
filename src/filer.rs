use actix::prelude::*;
use log::debug;
use sha1::{Digest, Sha1};

use std::net::Ipv4Addr;
use std::collections::BTreeMap;

use crate::{
    coordinator::{Coordinator, FilerResult},
    metadata::Info,
};

pub struct Filer {
    info: Info,
    pieces: BTreeMap<u32, Vec<u8>>,
    coordinator_addr: Addr<Coordinator>,
}

impl Filer {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>) -> Self {
        Self {
            info,
            pieces: BTreeMap::new(),
            coordinator_addr,
        }
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

pub enum FilerEvent {
    WrongSha1 { piece_index: u32 },
    IndexOutOfBounds { piece_index: u32 },
    FilerMsgErr,
    Accepted { piece_index: u32 },
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct WholePiece {
    pub index: u32,
    pub piece: Vec<u8>,
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

impl Handler<WholePiece> for Filer {
    type Result = ();

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);

        self.pieces.insert(msg.index, msg.piece);
        
        // TODO take the piece, arrange it, write it to disk where necessary
        // this includes creating the directory, files, etc
    }
}
