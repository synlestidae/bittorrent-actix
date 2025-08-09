use actix::prelude::*;
use log::{debug, error};

use std::{collections::HashMap, net::Ipv4Addr};

use crate::peer::{Peer, StoredPiece};

pub struct Filer {
    pieces: HashMap<u32, Vec<u8>>,
}

impl Filer {
    pub fn new() -> Self {
        Self {
            pieces: HashMap::new(),
        }
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PieceRequest {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub peer_addr: Addr<Peer>,
}

impl Handler<PieceRequest> for Filer {
    type Result = ();

    fn handle(&mut self, msg: PieceRequest, _ctx: &mut Context<Self>) -> Self::Result {
        match &self.pieces.get(&msg.index) {
            Some(piece_bytes) => {
                let block =
                    piece_bytes[msg.begin as usize..(msg.begin + msg.length) as usize].to_vec();

                let piece = StoredPiece {
                    index: msg.index,
                    begin: msg.begin,
                    block,
                };
                msg.peer_addr.do_send(piece);
            }
            None => {
                error!("Requested piece {} but don't have it yet", msg.index);
            }
        };
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct WholePiece {
    pub index: u32,
    pub piece: Vec<u8>,
}

impl Handler<WholePiece> for Filer {
    type Result = ();

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Storing piece {}", msg.index);

        self.pieces.insert(msg.index, msg.piece);
    }
}
