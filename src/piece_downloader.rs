use actix::prelude::*;

pub struct PieceDownloader {}

impl PieceDownloader {
    pub fn new() -> Self {
        Self {}
    }
}

impl Actor for PieceDownloader {
    type Context = Context<Self>;
}
