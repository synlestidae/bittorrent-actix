use crate::coordinator::{Coordinator, PeerDisconnect, PeerMessage};
use crate::metadata::PieceRequest;
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use log::{debug, error, trace};
use std::io;
use std::net::Ipv4Addr;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder, FramedRead};

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    coordinator_addr: Addr<Coordinator>,
    write_half: Option<OwnedWriteHalf>,
    assigned_pieces: Vec<u32>,
    writer: FramedWrite<BittorrentMessage, OwnedWriteHalf, BittorrentCodec>,
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
        let codec = BittorrentCodec {
            handshake_sent: false,
            handshake_received: false,
            info_hash: info_hash.clone(),
            peer_id: peer_id.clone(),
        };
        ctx.add_stream(FramedRead::new(read_half, codec.clone()));
        let writer = FramedWrite::new(write_half, codec, ctx);

        Self {
            ip_address,
            port,
            coordinator_addr,
            write_half: None,
            assigned_pieces: vec![],
            writer,
        }
    }
}

#[derive(Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct RecvMessage {
    pub msg: BittorrentMessage,
}

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
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: RecvMessage, _ctx: &mut Context<Self>) -> Self::Result {
        debug!(
            "Received RecvMessage from {}:{}",
            self.ip_address, self.port
        );
        let ip_address = self.ip_address;
        let port = self.port;
        self.coordinator_addr.do_send(PeerMessage {
            msg: msg.msg,
            ip_address,
            port,
        });

        Ok(())
    }
}

impl WriteHandler<PeerError> for Peer {
    // optional: override methods if needed
}

#[derive(Debug)]
pub enum PeerError {
    IOErr(io::Error),
    BittorrentMessageErr(BittorrentMessageError),
    NotConnected,
    Timeout,
}

impl From<io::Error> for PeerError {
    fn from(err: io::Error) -> Self {
        PeerError::IOErr(err)
    }
}

impl From<BittorrentMessageError> for PeerError {
    fn from(err: BittorrentMessageError) -> Self {
        PeerError::BittorrentMessageErr(err)
    }
}

#[derive(Debug)]
pub enum BittorrentMessageError {
    InvalidLength,
    UnknownId(u8),
    Malformed,
}

pub struct BittorrentHandshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl BittorrentHandshake {
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(49 + 20 + 20);
        buf.push(19);
        buf.extend_from_slice(b"BitTorrent protocol");
        buf.extend_from_slice(&[0; 8]);
        buf.extend_from_slice(&self.info_hash);
        buf.extend_from_slice(&self.peer_id);
        buf
    }

    pub fn len() -> usize {
        49 + 20 + 20
    }
}

#[derive(Debug)]
pub enum BittorrentMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have {
        piece_index: u32,
    },
    BitField {
        bitfield: Vec<u8>,
    },
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port {
        port: u16,
    },
}

impl BittorrentMessage {
    pub fn deserialize(data: &[u8]) -> Result<Self, BittorrentMessageError> {
        if data.len() < 4 {
            trace!("Expected length at least 4, got {}", data.len());
            return Err(BittorrentMessageError::InvalidLength);
        }
        let length = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;

        if length == 0 {
            return Ok(BittorrentMessage::KeepAlive);
        }

        if data.len() < 4 + length {
            trace!("Not enough data for total length: {}", data.len());
            return Err(BittorrentMessageError::InvalidLength);
        }

        let id = data[4];
        let payload = &data[5..4 + length];

        match id {
            0 => Ok(BittorrentMessage::Choke),
            1 => Ok(BittorrentMessage::Unchoke),
            2 => Ok(BittorrentMessage::Interested),
            3 => Ok(BittorrentMessage::NotInterested),
            4 => {
                if payload.len() != 4 {
                    return Err(BittorrentMessageError::InvalidLength);
                }
                let piece_index =
                    u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(BittorrentMessage::Have { piece_index })
            }
            5 => Ok(BittorrentMessage::BitField {
                bitfield: payload.to_vec(),
            }),
            6 => {
                if payload.len() != 12 {
                    return Err(BittorrentMessageError::InvalidLength);
                }
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                Ok(BittorrentMessage::Request {
                    index,
                    begin,
                    length,
                })
            }
            7 => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let block = payload[8..payload.len()].iter().cloned().collect();

                Ok(BittorrentMessage::Piece {
                    index,
                    begin,
                    block,
                })
            }
            8 => {
                let index = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let begin = u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let length = u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);

                Ok(BittorrentMessage::Cancel {
                    index,
                    begin,
                    length,
                })
            }
            9 => {
                let port = u16::from_be_bytes([payload[0], payload[1]]);

                Ok(BittorrentMessage::Port { port })
            }
            id => Err(BittorrentMessageError::UnknownId(id)),
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        match self {
            BittorrentMessage::KeepAlive => vec![0, 0, 0, 0],
            BittorrentMessage::Choke => vec![0, 0, 0, 1, 0],
            BittorrentMessage::Unchoke => vec![0, 0, 0, 1, 1],
            BittorrentMessage::Interested => vec![0, 0, 0, 1, 2],
            BittorrentMessage::NotInterested => vec![0, 0, 0, 1, 3],
            BittorrentMessage::Have { piece_index } => {
                let mut v: Vec<u8> = Vec::with_capacity(4 + 1 + 4);
                v.extend_from_slice(&(1u32 + 4u32).to_be_bytes());
                v.push(4);
                v.extend_from_slice(&piece_index.to_be_bytes());
                v
            }
            BittorrentMessage::BitField { bitfield } => {
                let mut v = Vec::with_capacity(4 + 1 + bitfield.len());
                v.extend_from_slice(&(1 + bitfield.len() as u32).to_be_bytes());
                v.push(5);
                v.extend_from_slice(bitfield);
                v
            }
            BittorrentMessage::Request {
                index,
                begin,
                length,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 12);
                v.extend_from_slice(&(1u32 + 12u32).to_be_bytes());
                v.push(6);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 2 * (4) + block.len());
                v.extend_from_slice(&(9u32 + block.len() as u32).to_be_bytes());
                v.push(7);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(block);
                v
            }
            BittorrentMessage::Cancel {
                index,
                begin,
                length,
            } => {
                let mut v = Vec::with_capacity(4 + 1 + 3 * 4);
                v.extend_from_slice(&(3u32 * 4u32).to_be_bytes());
                v.push(8);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
            BittorrentMessage::Port { port } => {
                let mut v = Vec::with_capacity(4 + 1 + 2);
                v.extend_from_slice(&(1u32 + 2u32).to_be_bytes());
                v.push(9);
                v.extend_from_slice(&port.to_be_bytes());
                v
            }
        }
    }
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub struct PeerControlEnvelope {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub messages: Vec<PeerControl>,
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub enum PeerControl {
    AssignPieces { assigned_pieces: Vec<u32> },
    ChokeThem,
    UnchokeThem,
    Interested,
    NotInterested,
    RequestPieces { piece_requests: Vec<PieceRequest> },
}

impl Handler<PeerControl> for Peer {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: PeerControl, _ctx: &mut Context<Self>) -> Self::Result {
        match msg {
            PeerControl::AssignPieces { assigned_pieces } => {
                debug!(
                    "Assigned {} piece(s) to {}:{}",
                    assigned_pieces.len(),
                    self.ip_address,
                    self.port,
                );
                self.assigned_pieces = assigned_pieces;
            }
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
        }

        Ok(())
    }
}

#[derive(Clone)]
struct BittorrentCodec {
    handshake_sent: bool,
    handshake_received: bool,
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl Decoder for BittorrentCodec {
    type Item = BittorrentMessage;
    type Error = PeerError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        trace!("Decoding message");
        if !self.handshake_received {
            trace!("Decoding handshake first");
            if src.len() < 1 {
                trace!("Have 0 bytes for handshake but need more");
                return Ok(None);
            }

            const RESERVED_LEN: usize = 8;
            const INFO_HASH_LEN: usize = 20;
            const PEER_ID_LEN: usize = 20;

            let pstr_len = src[0] as usize;
            let total_msg_len = 1 + pstr_len + RESERVED_LEN + INFO_HASH_LEN + PEER_ID_LEN;
            if src.len() < total_msg_len {
                trace!(
                    "Have {} byte(s) for handshake but need {}",
                    src.len(),
                    total_msg_len
                );
                return Ok(None);
            }

            // don't actually care, just read off the bytes and drop them
            drop(src.split_to(total_msg_len));

            trace!("Successfully decoded handshake");
            self.handshake_received = true;

            return Ok(None);
        }

        // check we have enough bytes
        if src.len() < 4 {
            trace!("Have {} bytes so do not know length", src.len());
            return Ok(None);
        }
        trace!("Reading message from {} bytes", src.len());
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
        let total_len = len as usize + 4;
        if src.len() < total_len {
            trace!("Have {} bytes but need at least {}", src.len(), total_len);

            return Ok(None);
        }

        // read the message
        trace!("Reading {} bytes", total_len);
        let msg = BittorrentMessage::deserialize(&src.split_to(total_len))?;

        trace!("Successfully deserialised message");
        Ok(Some(msg))
    }
}

impl Encoder<BittorrentMessage> for BittorrentCodec {
    type Error = PeerError;

    fn encode(&mut self, item: BittorrentMessage, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if !self.handshake_sent {
            trace!("Sending handshake first");

            let handshake = BittorrentHandshake {
                info_hash: self.info_hash,
                peer_id: self.peer_id,
            };
            dst.extend_from_slice(&handshake.serialize());

            self.handshake_sent = true;
        }
        trace!("Sending {:?}", item);

        dst.extend_from_slice(&item.serialize());

        Ok(())
    }
}

impl StreamHandler<Result<BittorrentMessage, PeerError>> for Peer {
    fn handle(
        &mut self,
        msg_result: Result<BittorrentMessage, PeerError>,
        ctx: &mut Self::Context,
    ) {
        match msg_result {
            Err(err) => {
                error!("Error receiving message: {:?}", err);
                self.coordinator_addr.do_send(PeerDisconnect {
                    ip_address: self.ip_address,
                    port: self.port,
                });
            }
            Ok(msg) => {
                ctx.address().do_send(RecvMessage { msg });
            }
        }
    }
}

impl Actor for Peer {
    type Context = Context<Self>;

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        debug!("Disconnecting from peer {}:{}", self.ip_address, self.port);
    }
}
