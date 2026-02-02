use core::error;
use std::net::Ipv4Addr;

use actix::prelude::*;
use log::{debug, error};
use tokio::io;

use crate::{
    bittorrent::{BittorrentMessage, Info, Metadata},
    filer::{
        file_writer::{self, FileWriter, FileWriterError},
        piece_file,
        piece_reader::{self, PieceReader},
    },
};

use super::PieceFile;

pub struct Filer {
    piece_file: PieceFile,
    metadata: Metadata,
}

impl Filer {
    pub fn new(metadata: Metadata, piece_file: PieceFile) -> Self {
        Self {
            metadata,
            piece_file,
        }
    }

    fn schedule_file_writer(&self, ctx: &mut <Self as Actor>::Context) {
        let piece_reader = PieceReader::new(self.piece_file.clone());
        let mut file_writer = FileWriter::new(self.metadata.clone(), piece_reader);

        ctx.spawn(Box::pin(
            async move {
                file_writer
                    .write_to_disk()
                    .await
                    .expect("unable to write to file");
            }
            .into_actor(self),
        ));
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<PieceReport, io::Error>")]
pub struct WholePiece {
    pub index: u32,
    pub data: Vec<u8>,
}

pub struct PieceReport {
    pub highest_piece: i32,
}

impl Handler<WholePiece> for Filer {
    type Result = ResponseActFuture<Self, Result<PieceReport, io::Error>>;

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);
        let mut piece_file = self.piece_file.clone();

        return Box::pin(
            async move {
                piece_file.write(msg.index, msg.data).await.unwrap();

                Ok(())
            }
            .into_actor(self)
            .map(|result: Result<(), _>, actor, _ctx| {
                if let Err(err) = result {
                    return Err(err);
                }

                Ok(PieceReport {
                    highest_piece: actor.piece_file.highest_piece(),
                })
            }),
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

#[derive(Message)]
#[rtype(result = "Result<(), FileWriterError>")]
pub struct Finish {}

impl Handler<Finish> for Filer {
    type Result = ResponseActFuture<Self, Result<(), FileWriterError>>;

    fn handle(&mut self, _msg: Finish, _ctx: &mut Context<Self>) -> Self::Result {
        let metadata = self.metadata.clone();
        let piece_file = self.piece_file.clone();

        Box::pin(
            async {
                let piece_reader = PieceReader::new(piece_file);
                let mut file_writer = FileWriter::new(metadata, piece_reader);

                file_writer.write_to_disk().await?;

                Ok(())
            }
            .into_actor(self),
        )
    }
}
