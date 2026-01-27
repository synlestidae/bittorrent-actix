use actix::prelude::*;

pub struct PieceUploader {
}

impl PieceUploader {
    pub fn new() -> Self {
        Self {
        }
    }
}

impl Actor for PieceUploader {
    type Context = Context<Self>;
}
