use std::net::Ipv4Addr;

use actix::prelude::*;
use log::debug;
use tokio::io;

use crate::commander::CommandSet;
use crate::metadata::Info;
use crate::peer::{PeerControl, PeerControlEnvelope};
use super::PieceFile;

pub struct Filer {
    info: Info,
    piece_file: PieceFile,
}

const MAGIC_NUMBER: &'static [u8; 8] = b"PIECESv1";

const CHANNEL_DEPTH: usize = 100;

impl Filer {
    pub async fn new(info: Info) -> Result<Self, io::Error> {
        let filename = info.filename();
        let piece_file = PieceFile::open(&filename).await?;

        Ok(Self { info, piece_file })
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
#[rtype(result = "Result<CommandSet, io::Error>")]
pub struct RequestPieceEnvelope {
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

impl Handler<RequestPieceEnvelope> for Filer {
    type Result = ResponseActFuture<Self, Result<CommandSet, io::Error>>;

    fn handle(&mut self, msg: RequestPieceEnvelope, _ctx: &mut Context<Self>) -> Self::Result {
        let mut piece_file = self.piece_file.clone();

        return Box::pin(
            async move {
                let mut command_set = CommandSet::default();
                for request_piece in msg.request_pieces.into_iter() {
                    match piece_file
                        .read(
                            request_piece.index,
                            request_piece.begin,
                            request_piece.length,
                        )
                        .await?
                    {
                        Some(block) => {
                            command_set.peer_control.push(PeerControlEnvelope {
                                ip_address: request_piece.ip_address,
                                port: request_piece.port,
                                messages: vec![PeerControl::UploadPieceToThem {
                                    index: request_piece.index,
                                    begin: request_piece.begin,
                                    block,
                                }],
                            });
                        }
                        None => {}
                    }
                }

                Ok(command_set)
            }
            .into_actor(self),
        );
    }
}

struct ReadPiece {
    index: u32,
    offset: u32,
    length: u32,
}

struct WritePiece {
    index: u32,
    data: Vec<u8>,
}

struct WritePieceDone {
    index: u32,
}
