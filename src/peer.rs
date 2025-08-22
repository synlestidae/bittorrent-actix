use crate::coordinator::{Coordinator, PeerMessage};
use crate::metadata::Info;
use crate::piece_tracker::PieceTracker;
use actix::prelude::*;
use log::{debug, error, trace};
use std::io;
use std::net::Ipv4Addr;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio_util::bytes::BytesMut;
use tokio_util::codec::{Decoder, FramedRead};

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    coordinator_addr: Addr<Coordinator>,
    write_half: Option<OwnedWriteHalf>,
    assigned_pieces: Vec<u32>,
    info: Info,
    pieces: PieceTracker,
}

#[derive(Clone)]
struct RequestedPiece {
    index: u32,
    begin: u32,
    length: u32,
}

const MAX_PIECE_REQUESTS: usize = 10;

impl Peer {
    pub fn new(
        ip_address: Ipv4Addr,
        port: u16,
        coordinator_addr: Addr<Coordinator>,
        info: &Info,
    ) -> Self {
        Self {
            ip_address,
            port,
            coordinator_addr,
            write_half: None,
            info: info.clone(),
            assigned_pieces: vec![],
            pieces: PieceTracker::new(info.pieces_length),
        }
    }

    fn handshake(
        &self,
        handshake: BittorrentHandshake,
    ) -> ResponseActFuture<Self, Result<(), PeerError>> {
        let ip_address = self.ip_address;
        let port = self.port;

        Box::pin(
            async move {
                let mut tcp_stream = TcpStream::connect(&(ip_address, port)).await?;

                tcp_stream.write_all(&handshake.serialize()).await?;

                let mut pstr_len = [0u8; 1];
                tcp_stream.read_exact(&mut pstr_len).await?;

                let mut pstr_data = vec![0u8; pstr_len[0] as usize];
                tcp_stream.read_exact(&mut pstr_data).await?;
                let pstr = String::from_utf8_lossy(&pstr_data).to_string();

                debug!("Protocol string: {}", pstr);

                let mut reserved_data = [0u8; 8];
                let mut info_hash_data = [0u8; 20];
                let mut peer_id = [0u8; 20];

                tcp_stream.read_exact(&mut reserved_data).await?;
                tcp_stream.read_exact(&mut info_hash_data).await?;
                tcp_stream.read_exact(&mut peer_id).await?;

                debug!("Handshake bytes written for {}:{}", ip_address, port);

                Ok(tcp_stream)
            }
            .into_actor(self)
            .map(
                move |tcp_stream_result, actor, ctx| match tcp_stream_result {
                    Ok(tcp_stream) => {
                        let (read_half, write_half) = tcp_stream.into_split();
                        ctx.add_stream(FramedRead::new(read_half, BittorrentCodec));
                        actor.write_half = Some(write_half);

                        Ok(())
                    }
                    Err(err) => {
                        error!("Error connecting to {}:{} - {:?}", ip_address, port, err);

                        Err(err)
                    }
                },
            ),
        )
    }

    fn request_pieces(&mut self) -> ResponseActFuture<Self, Result<(), PeerError>> {
        let messages: Vec<_> = self
            .pieces
            .next_pieces(&self.assigned_pieces, MAX_PIECE_REQUESTS)
            .iter()
            .map(|piece_request| BittorrentMessage::Request {
                index: piece_request.index,
                begin: piece_request.begin,
                length: piece_request.length,
            })
            .collect();

        Box::pin(
            match self.write_half.take() {
                Some(mut write_half) => {
                    async move {
                        debug!("Sending {} message(s)", messages.len());
                        for msg in messages.into_iter() {
                            write_half.write_all(&msg.serialize()).await?;
                        }

                        Ok(())
                    }
                }
                None => unimplemented!("Never mind") 
            }
            .into_actor(self),
        )
    }
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub struct Handshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handler<Handshake> for Peer {
    type Result = ResponseActFuture<Self, Result<(), PeerError>>;

    fn handle(&mut self, msg: Handshake, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received Handshake");

        self.handshake(BittorrentHandshake {
            info_hash: msg.info_hash,
            peer_id: msg.peer_id,
        })
    }
}

#[derive(Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct SendMessage {
    pub msg: BittorrentMessage,
}

#[derive(Message)]
#[rtype(result = "()")]
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
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: RecvMessage, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received RecvMessage");
        let ip_address = self.ip_address;
        let port = self.port;
        let coordinator_addr = self.coordinator_addr.clone();

        if let BittorrentMessage::Choke = msg.msg {
            self.pieces.clear_requests();
        }

        Box::pin(
            async move {
                match coordinator_addr
                    .send(PeerMessage {
                        msg: msg.msg,
                        ip_address,
                        port,
                    })
                    .await
                {
                    Ok(_) => (),
                    Err(err) => {
                        error!("Error sending peer message to coordinator: {}", err);
                    }
                }
            }
            .into_actor(self),
        )
    }
}

#[derive(Debug)]
pub enum PeerError {
    IOErr(io::Error),
    BittorrentMessageErr(BittorrentMessageError),
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
    UnknownId,
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

    pub fn deserialize(data: &[u8]) -> Result<Self, BittorrentMessageError> {
        if data.len() < 68 {
            return Err(BittorrentMessageError::UnknownId);
        }
        let pstrlen = data[0] as usize;
        if pstrlen != 19 {
            return Err(BittorrentMessageError::UnknownId);
        }
        if &data[1..20] != b"BitTorrent protocol" {
            return Err(BittorrentMessageError::Malformed);
        }
        let info_hash = data[28..48]
            .try_into()
            .map_err(|_| BittorrentMessageError::InvalidLength)?;
        let peer_id = data[48..68]
            .try_into()
            .map_err(|_| BittorrentMessageError::InvalidLength)?;
        Ok(Self { info_hash, peer_id })
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
            _ => Err(BittorrentMessageError::UnknownId),
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
                let mut v = Vec::with_capacity(9 + block.len());
                v.extend_from_slice(&(9u32 + block.len() as u32).to_be_bytes());
                v.push(7);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(block);
                v
            }
        }
    }
}

#[derive(Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct AssignPieces {
    pub assigned_pieces: Vec<u32>,
}

impl Handler<AssignPieces> for Peer {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: AssignPieces, _ctx: &mut Context<Self>) -> Self::Result {
        self.assigned_pieces = msg.assigned_pieces;

        Ok(())
    }
}

struct BittorrentCodec;

impl Decoder for BittorrentCodec {
    type Item = BittorrentMessage;
    type Error = PeerError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // check we have enough bytes
        if src.len() < 4 {
            trace!("Have {} bytes so do not know length", src.len());
            return Ok(None);
        }
        let len = u32::from_be_bytes([src[0], src[1], src[2], src[3]]);
        let total_len = len as usize + 4;
        if src.len() < total_len {
            trace!("Have {} bytes but need at least{}", src.len(), total_len);

            return Ok(None);
        }

        // read the message
        let msg = BittorrentMessage::deserialize(&src.split_to(total_len))?;
        Ok(Some(msg))
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
        debug!("Removing peer {}:{}", self.ip_address, self.port);
    }
}
