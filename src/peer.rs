use crate::coordinator::{Coordinator, PeerMessage};
use crate::metadata::Info;
use actix::prelude::*;
use bitvec::vec::BitVec;
use log::{debug, error, trace};
use std::cmp::min;
use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::net::Ipv4Addr;
use std::net::TcpStream as StdTcpStream;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    coordinator_addr: Addr<Coordinator>,
    tcp_stream: Option<StdTcpStream>,
    read_loop: Option<JoinHandle<()>>,
    assigned_pieces: BitVec,
    info: Info,
    pieces: HashMap<u32, Vec<StoredPiece>>,
    pieces_we_requested: Vec<RequestedPiece>,
}

#[derive(Clone)]
struct RequestedPiece {
    index: u32,
    begin: u32,
    length: u32,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct StoredPiece {
    pub index: u32,
    pub begin: u32,
    pub block: Vec<u8>,
}

const MAX_PIECE_REQUESTS: usize = 10;

const PIECE_REQUEST_SIZE: u32 = 16_384;

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
            tcp_stream: None,
            read_loop: None,
            assigned_pieces: BitVec::repeat(false, info.pieces.len()),
            info: info.clone(),
            pieces: HashMap::new(),
            pieces_we_requested: Vec::new(),
        }
    }

    fn handshake(
        &self,
        handshake: BittorrentHandshake,
    ) -> ResponseActFuture<Self, Result<(), NetworkError>> {
        let ip_address = self.ip_address;
        let port = self.port;

        return Box::pin(
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
            .map(move |tcp_stream_result, actor, ctx| match tcp_stream_result {
                Ok(tcp_stream) => {
                    let std_tcp_stream = tcp_stream.into_std().unwrap();
                    let tcp_stream_read = clone_std_tcp_stream(&std_tcp_stream);

                    ctx.spawn(read_loop(tcp_stream_read, ctx.address()).into_actor(actor));

                    actor.tcp_stream = Some(std_tcp_stream);


                    Ok(())
                }
                Err(err) => {
                    error!("Error connecting to {}:{} - {:?}", ip_address, port, err);

                    Err(err)
                },
            }),
        );
    }

    fn socket(&self) -> Result<TcpStream, NetworkError> {
        if let Some(std_tcp_stream) = &self.tcp_stream {
            return Ok(TcpStream::from_std(std_tcp_stream.try_clone().unwrap()).unwrap());
        }

        Err(NetworkError::NotConnected)
    }
}

impl Actor for Peer {
    type Context = Context<Self>;

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        debug!("Removing peer {}:{}", self.ip_address, self.port);

        if let Some(Err(err)) = self
            .tcp_stream
            .take()
            .map(|tcp_stream| tcp_stream.shutdown(std::net::Shutdown::Both))
        {
            error!("Error shutting down stream: {}", err);
        }
        if let Some(Err(err)) = self.read_loop.take().map(|join_handle| join_handle.join()) {
            error!("Error shutting down read loop: {:?}", err);
        }
    }
}

#[derive(Message)]
#[rtype(result = "Result<(), NetworkError>")]
pub struct Handshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

impl Handler<Handshake> for Peer {
    type Result = ResponseActFuture<Self, Result<(), NetworkError>>;

    fn handle(&mut self, msg: Handshake, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received Handshake");

        self.handshake(BittorrentHandshake { info_hash: msg.info_hash, peer_id: msg.peer_id })

        /*return Box::pin(
            async move {
                let tcp_stream =
                    TcpStream::connect_timeout(&(ip_address, port).into(), default_timeout)?;
                tcp_stream.set_nonblocking(true);

                Ok(tcp_stream)
            }
            .into_actor(self)
            .map(move |result: Result<TcpStream, NetworkError>, _, ctx| {
                let addr = ctx.address();
                match result {
                    Ok(tcp_stream) => {
                        read_loop(tcp_stream.try_clone().unwrap(), addr);

                        debug!("Peer {}:{} is ready", ip_address, port);

                        coordinator_addr.do_send(HandshakeEnd {
                            ip_address: ip_address,
                            port: port,
                            error: None,
                        });
                    }
                    Err(err) => {
                        coordinator_addr.do_send(HandshakeEnd {
                            ip_address: ip_address,
                            port: port,
                            error: Some(err.into()),
                        });

                        error!("Error connecting to peer {}:{}", ip_address, port);
                    }
                }
            }),
        );*/
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RecvMessage {
    bittorrent_message: BittorrentMessage,
}

impl Handler<RecvMessage> for Peer {
    type Result = ();

    fn handle(&mut self, msg: RecvMessage, _ctx: &mut Context<Self>) -> Self::Result {
        self.coordinator_addr.do_send(PeerMessage {
            ip_address: self.ip_address,
            port: self.port,
            bittorrent_message: msg.bittorrent_message,
        });
    }
}

async fn read_loop(mut tcp_stream: TcpStream, addr: Addr<Peer>) {
    loop {
        let mut bytes = vec![0u8; 4];
        tcp_stream.read_exact(&mut bytes).await.unwrap();
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        bytes.resize(len + 4, 0u8);

        trace!("Reading message of length {}", len);

        match tcp_stream.read_exact(&mut bytes[4..len]).await {
            Ok(b) => {
                debug!("Read {} bytes", b);
            }
            Err(err) => {
                error!("Read error: {}", err);
                return;
            }
        };
        let bittorrent_message = match BittorrentMessage::deserialize(&bytes) {
            Ok(m) => m,
            Err(err) => {
                error!("Peer message error: {:?}", err);
                return;
            }
        };

        trace!("Got message {:?}", bittorrent_message);
        addr.do_send(RecvMessage { bittorrent_message });

        return;
    }
}

fn clone_std_tcp_stream(tcp_stream: &StdTcpStream) -> TcpStream {
    let std_stream = tcp_stream.try_clone().unwrap();
    TcpStream::from_std(std_stream).unwrap()
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), NetworkError>")]
pub struct HavePiece {
    pub piece_index: u32,
}

impl Handler<HavePiece> for Peer {
    type Result = Result<(), NetworkError>;

    fn handle(&mut self, msg: HavePiece, _ctx: &mut Context<Self>) -> Self::Result {
        let bittorrent_message = BittorrentMessage::Have {
            piece_index: msg.piece_index,
        };

        match self.tcp_stream {
            Some(ref mut tcp_stream) => {
                tcp_stream.write_all(&bittorrent_message.serialize())?;
            }
            None => {
                panic!("No connection available");
            }
        };

        Ok(())
    }
}

#[derive(Debug)]
pub enum NetworkError {
    IOErr(io::Error),
    NotConnected,
}

impl From<io::Error> for NetworkError {
    fn from(err: io::Error) -> Self {
        NetworkError::IOErr(err)
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
                v.extend_from_slice(&block);
                v
            }
        }
    }
}

#[derive(Message)]
#[rtype(result = "Result<(), NetworkError>")]
pub struct AssignPieces {
    pub assigned_pieces: BitVec,
}

impl Handler<AssignPieces> for Peer {
    type Result = Result<(), NetworkError>;

    fn handle(&mut self, msg: AssignPieces, _ctx: &mut Context<Self>) -> Self::Result {
        self.assigned_pieces = msg.assigned_pieces;

        if self.pieces_we_requested.len() >= MAX_PIECE_REQUESTS {
            return Ok(());
        }

        let tcp_stream = match &mut self.tcp_stream {
            Some(tcp_stream) => tcp_stream,
            None => panic!("Missing TCP stream"),
        };

        // work out which existing pieces need a request
        let pieces_can_finish: Vec<u32> = self
            .pieces_we_requested
            .iter()
            .map(|p| match self.assigned_pieces.get(p.index as usize) {
                Some(b) => (p.index, *b),
                None => (p.index, false),
            })
            .filter(|(_, b)| *b)
            .map(|(i, _)| i)
            .collect();

        for piece_index in pieces_can_finish.iter() {
            let stored_pieces = self.pieces.get(piece_index).unwrap();
            let missing_pieces = get_missing_pieces(
                &self.pieces_we_requested,
                stored_pieces,
                self.info.pieces_length,
            );

            for missing_piece in missing_pieces.into_iter() {
                if self.pieces_we_requested.len() >= MAX_PIECE_REQUESTS {
                    return Ok(());
                }
                let message = BittorrentMessage::Request {
                    index: missing_piece.index,
                    begin: missing_piece.begin,
                    length: missing_piece.length,
                };
                tcp_stream.write_all(&message.serialize())?;
                self.pieces_we_requested.push(missing_piece);
            }
        }
        // work out which new pieces to request
        for piece_index in 0..self.assigned_pieces.len() {
            let stored_pieces = self.pieces.get(&(piece_index as u32)).unwrap();
            let missing_pieces = get_missing_pieces(
                &self.pieces_we_requested,
                &stored_pieces,
                PIECE_REQUEST_SIZE,
            );

            for missing_piece in missing_pieces.into_iter() {
                if self.pieces_we_requested.len() >= MAX_PIECE_REQUESTS {
                    return Ok(());
                }
                let message = BittorrentMessage::Request {
                    index: missing_piece.index,
                    begin: missing_piece.begin,
                    length: missing_piece.length,
                };
                tcp_stream.write_all(&message.serialize())?;
                self.pieces_we_requested.push(missing_piece);
            }
        }

        Ok(())
    }
}

fn get_missing_pieces(
    pieces_we_requested: &[RequestedPiece],
    stored_pieces: &[StoredPiece],
    piece_length: u32,
) -> Vec<RequestedPiece> {
    let mut pieces: Vec<RequestedPiece> = stored_pieces
        .iter()
        .map(|stored_piece| RequestedPiece {
            index: stored_piece.index,
            begin: stored_piece.begin,
            length: stored_piece.block.len() as u32,
        })
        .chain(pieces_we_requested.iter().cloned())
        .collect();

    pieces
        .sort_by(|piece0, piece1| (piece0.index, piece1.begin).cmp(&(piece1.index, piece1.begin)));

    let mut new_pieces = Vec::new();

    for i in 0..pieces.len() - 1 {
        let piece0 = &pieces[i];
        let piece1 = &pieces[i + 1];

        if piece0.index == piece1.index && piece0.begin + piece0.length < piece1.begin {
            let mut gap = fill_gap(piece0, piece1.begin, piece_length);
            new_pieces.append(&mut gap);
        }

        if piece0.index != piece1.index && piece0.begin + piece0.length < piece_length {
            let mut gap = fill_gap(piece0, piece_length, piece_length);
            new_pieces.append(&mut gap);
        }
    }

    new_pieces
}

fn fill_gap(piece0: &RequestedPiece, until_offset: u32, piece_length: u32) -> Vec<RequestedPiece> {
    let mut new_pieces = Vec::new();
    let mut offset: u32 = piece0.begin;
    let length = min(PIECE_REQUEST_SIZE, piece_length);

    // Add pieces until gap closes
    while offset < until_offset {
        new_pieces.push(RequestedPiece {
            index: piece0.index,
            begin: offset,
            length,
        });

        offset += PIECE_REQUEST_SIZE;
    }

    return new_pieces;
}

impl Handler<StoredPiece> for Peer {
    type Result = ();

    fn handle(&mut self, msg: StoredPiece, ctx: &mut Context<Self>) -> Self::Result {
        let bittorrent_message = BittorrentMessage::Piece {
            index: msg.index,
            begin: msg.begin,
            block: msg.block,
        };
        let bytes = bittorrent_message.serialize();

        if let Some(ref mut tcp_stream) = self.tcp_stream {
            let result = tcp_stream.write_all(&bytes);
            if let Err(err) = result {
                error!("Error writing piece: {}", err);
                ctx.stop();
            }
        }
    }
}
