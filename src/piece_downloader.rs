use actix::prelude::*;

use crate::event::EventHandler;

pub struct PieceDownloader {
    piece_index: u32,
    event_handler: Addr<EventHandler>,
}

impl PieceDownloader {
    pub fn new(piece_index: u32, event_handler: Addr<EventHandler>) -> Self {
        Self {
            piece_index,
            event_handler,
        }
    }
}

impl Actor for PieceDownloader {
    type Context = Context<Self>;
}
