use std::net::Ipv4Addr;

use actix::prelude::*;
use log::debug;
use tokio::io;

use crate::bittorrent::{BittorrentMessage, Info};

use super::PieceFile;

pub struct Filer {
    info: Info,
    piece_file: PieceFile,
}

impl Filer {
    pub async fn new(info: Info, piece_file: PieceFile) -> Result<Self, io::Error> {
        Ok(Self { info, piece_file })
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<(), io::Error>")]
pub struct WholePiece {
    pub index: u32,
    pub data: Vec<u8>,
}

impl Handler<WholePiece> for Filer {
    type Result = ResponseActFuture<Self, Result<(), io::Error>>;

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);
        let mut piece_file = self.piece_file.clone();

        return Box::pin(
            async move {
                piece_file.write(msg.index, msg.data).await?;

                Ok(())
            }
            .into_actor(self),
        );
    }
}

#[derive(Message)]
#[rtype(result = "Result<Vec<(Ipv4Addr, u16, BittorrentMessage)>, io::Error>")]
pub struct RequestPieces {
    pub request_pieces: Vec<RequestPiece>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RequestPiece {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

impl Handler<RequestPieces> for Filer {
    type Result =
        ResponseActFuture<Self, Result<Vec<(Ipv4Addr, u16, BittorrentMessage)>, io::Error>>;

    fn handle(&mut self, msg: RequestPieces, _ctx: &mut Context<Self>) -> Self::Result {
        let mut piece_file = self.piece_file.clone();

        Box::pin(
            async move {
                let mut messages = Vec::new();
                for request in msg.request_pieces.into_iter() {
                    if let Some(block) = piece_file
                        .read(request.index, request.begin, request.length)
                        .await?
                    {
                        messages.push((
                            request.ip_address,
                            request.port,
                            BittorrentMessage::Piece {
                                index: request.index,
                                begin: request.begin,
                                block,
                            },
                        ));
                    }
                }

                Ok(messages)
            }
            .into_actor(self),
        )
    }
}
