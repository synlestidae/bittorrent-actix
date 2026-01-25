use crate::bittorrent::BittorrentCodec;
use crate::bittorrent::BittorrentMessage;
use crate::bittorrent::DecoderError;
use crate::coordinator::RecvMessage;
use crate::coordinator::{Coordinator, PeerDisconnect};
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use log::{debug, error};
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
struct NotifyRecv {}

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

impl Handler<NotifyRecv> for Peer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, _: NotifyRecv, _ctx: &mut Context<Self>) -> Self::Result {
        let ip_address = self.ip_address;
        let port = self.port;

        let messages = self.messages.drain(..).collect();
        let recv_message = RecvMessage {
            ip_address,
            port,
            messages,
        };

        Box::pin(
            self.coordinator_addr
                .send(recv_message)
                .into_actor(self)
                .map(|result, _, _| {
                    if let Err(err) = result {
                        error!("Error sending to coordinator: {}", err);
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
pub struct SendMessages {
    pub messages: Vec<BittorrentMessage>,
}

impl Handler<SendMessages> for Peer {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, envelope: SendMessages, _ctx: &mut Context<Self>) -> Self::Result {
        for msg in envelope.messages.into_iter() {
            self.writer.write(msg);
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
                ctx.notify(NotifyRecv {});
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
