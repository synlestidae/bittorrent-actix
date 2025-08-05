use actix::prelude::*;
use log::error;

use std::collections::HashMap;

#[derive(Default)]
pub struct Filer {
    pieces: HashMap<u32, Vec<u8>>,
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
}

pub struct Piece {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
}

impl Handler<PieceRequest> for Filer {
    type Result = Option<Piece>;

    fn handle(&mut self, msg: PieceRequest,  ctx: &mut Context<Self>) -> Self::Result {
        match &self.pieces.get(&msg.index) {
            Some(piece_bytes) => {
                Some(Piece {
                    index: msg.index,
                    begin: msg.begin,
                    block: piece_bytes[(msg.begin as usize)..(msg.length as usize)].to_vec(),
                })
            },
            None => {
                error!("Requested piece {} but don't have it yet", msg.index);
                None
            }
        }
    }
}
