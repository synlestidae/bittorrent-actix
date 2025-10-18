use crate::bittorrent::{BittorrentCodec, BittorrentMessage, DecoderError};
use crate::commander::{CommandSet, PeerChangeEnvelope};
use crate::coordinator::{Coordinator, PeerDisconnect};
use crate::metadata::PieceRequest;
use crate::peer_state::PeerChange;
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use log::{debug, error, trace};
use std::net::Ipv4Addr;
use std::string::ParseError;
use tokio::io;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio_util::codec::FramedRead;

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    coordinator_addr: Addr<Coordinator>,
    write_half: Option<OwnedWriteHalf>,
    writer: FramedWrite<BittorrentMessage, OwnedWriteHalf, BittorrentCodec>,
    messages: Vec<BittorrentMessage>,
}

impl Peer {
    pub fn new(
        ip_address: Ipv4Addr,
        port: u16,
        coordinator_addr: Addr<Coordinator>,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
        tcp_stream: TcpStream,
        ctx: &mut Context<Self>,
    ) -> Self {
        let (read_half, write_half) = tcp_stream.into_split();
        let codec = BittorrentCodec::new(info_hash, peer_id);
        ctx.add_stream(FramedRead::new(read_half, codec.clone()));
        let writer = FramedWrite::new(write_half, codec, ctx);

        Self {
            ip_address,
            port,
            coordinator_addr,
            write_half: None,
            writer,
            messages: Vec::new(),
        }
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RecvMessage {}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub struct HavePiece {
    pub piece_index: u32,
}

impl Handler<HavePiece> for Peer {
    type Result = ResponseActFuture<Self, Result<(), PeerError>>;

    fn handle(&mut self, msg: HavePiece, _ctx: &mut Context<Self>) -> Self::Result {
        let msg = BittorrentMessage::Have {
            piece_index: msg.piece_index,
        };
        let bytes = msg.serialize();
        match self.write_half.take() {
            Some(mut write_half) => Box::pin(
                async move {
                    write_half.write_all(&bytes).await?;
                    Ok(())
                }
                .into_actor(self),
            ),
            None => Box::pin(async { Ok(()) }.into_actor(self)),
        }
    }
}

impl Handler<RecvMessage> for Peer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, _: RecvMessage, _ctx: &mut Context<Self>) -> Self::Result {
        let ip_address = self.ip_address;
        let port = self.port;
        let mut peer_change: PeerChange = Default::default();
        let mut command_set: CommandSet = Default::default();

        for msg in self.messages.drain(..) {
            peer_change.handle_message(msg)
        }

        command_set.peer_change.push(PeerChangeEnvelope {
            ip_address,
            port,
            peer_change,
        });

        Box::pin(
            self.coordinator_addr
                .send(command_set)
                .into_actor(self)
                .map(|result, _, _| {
                    if let Err(err) = result {
                        error!("Error sending command set: {}", err);
                    }
                }),
        )
    }
}

impl WriteHandler<io::Error> for Peer {
    // TODO this sort of error should shut the peer down
}

#[derive(Debug)]
pub enum PeerError {
    IOErr(io::Error),
    BittorrentMessageErr(ParseError),
    NotConnected,
    Timeout,
}

impl From<io::Error> for PeerError {
    fn from(err: io::Error) -> Self {
        PeerError::IOErr(err)
    }
}

impl From<ParseError> for PeerError {
    fn from(err: ParseError) -> Self {
        PeerError::BittorrentMessageErr(err)
    }
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub struct PeerControlEnvelope {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub messages: Vec<PeerControl>,
}

#[derive(Message, Debug, PartialEq, Eq)]
#[rtype(result = "Result<(), PeerError>")]
pub enum PeerControl {
    ChokeThem,
    UnchokeThem,
    Interested,
    NotInterested,
    RequestPieces {
        piece_requests: Vec<PieceRequest>,
    },
    UploadPieceToThem {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    HavePiece {
        index: u32,
    },
    Bitfield {
        bitfield: Vec<u8>,
    },
}

impl Handler<PeerControlEnvelope> for Peer {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, envelope: PeerControlEnvelope, _ctx: &mut Context<Self>) -> Self::Result {
        for msg in envelope.messages.into_iter() {
            trace!("Received peer control {:?}", msg);
            match msg {
                PeerControl::ChokeThem => {
                    debug!("Sending Choke to {}:{}", self.ip_address, self.port);
                    self.writer.write(BittorrentMessage::Choke);
                }
                PeerControl::UnchokeThem => {
                    debug!("Sending Unchoke to {}:{}", self.ip_address, self.port);
                    self.writer.write(BittorrentMessage::Unchoke);
                }
                PeerControl::Interested => {
                    debug!("Sending Interested to {}:{}", self.ip_address, self.port);
                    self.writer.write(BittorrentMessage::Interested);
                }
                PeerControl::NotInterested => {
                    debug!("Sending NotInterested to {}:{}", self.ip_address, self.port);
                    self.writer.write(BittorrentMessage::NotInterested);
                }
                PeerControl::RequestPieces { piece_requests } => {
                    debug!("Sending PieceRequests to {}:{}", self.ip_address, self.port);
                    for piece_request in piece_requests.into_iter() {
                        self.writer.write(BittorrentMessage::Request {
                            index: piece_request.index,
                            begin: piece_request.begin,
                            length: piece_request.length,
                        });
                    }
                }
                PeerControl::UploadPieceToThem {
                    index,
                    begin,
                    block,
                } => {
                    self.writer.write(BittorrentMessage::Piece {
                        index,
                        begin,
                        block,
                    });
                }
                PeerControl::HavePiece { index } => {
                    self.writer
                        .write(BittorrentMessage::Have { piece_index: index });
                }
                PeerControl::Bitfield { bitfield } => {
                    self.writer
                        .write(BittorrentMessage::Bitfield { bitfield: bitfield });
                }
            }
        }

        Ok(())
    }
}

impl StreamHandler<Result<Vec<BittorrentMessage>, DecoderError>> for Peer {
    fn handle(
        &mut self,
        msg_result: Result<Vec<BittorrentMessage>, DecoderError>,
        ctx: &mut Self::Context,
    ) {
        match msg_result {
            Err(err) => {
                error!("Error receiving message: {}", err);
                ctx.spawn(
                    self.coordinator_addr
                        .send(PeerDisconnect {
                            ip_address: self.ip_address,
                            port: self.port,
                        })
                        .into_actor(self)
                        .map(|result, _, _| {
                            if let Err(err) = result {
                                error!("Error disconnecting peer: {}", err);
                            }
                        }),
                );
            }
            Ok(mut msgs) => {
                self.messages.append(&mut msgs);
                ctx.spawn(ctx.address().send(RecvMessage {}).into_actor(self).map(
                    |result, _, _| {
                        if let Err(err) = result {
                            error!("Error signaling RecvMessage: {}", err);
                        }
                    },
                ));
            }
        }
    }
}

impl Actor for Peer {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.set_mailbox_capacity(100);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        debug!("Disconnecting from peer {}:{}", self.ip_address, self.port);
    }
}
