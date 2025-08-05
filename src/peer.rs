use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use log::{debug, error, trace};
use std::net::Ipv4Addr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
//use tokio::io;
//use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tracker::Tracker;
use crate::filer::{Filer, PieceRequest};
use actix::prelude::*;
//use tokio::net::TcpStream;
//use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    tracker_addr: Addr<Tracker>,
    filer: Addr<Filer>,
    write_half: Option<Rc<Mutex<OwnedWriteHalf>>>,
    read_loop: Option<SpawnHandle>,
    state: State,
}

impl Peer {
    pub fn new(ip_address: Ipv4Addr, port: u16, tracker_addr: Addr<Tracker>) -> Self {
        Self {
            ip_address,
            port,
            tracker_addr,
            write_half: None,
            read_loop: None,
            state: State::new(),
        }
    }
}

struct State {
    they_have: BitVec<u8, Msb0>,
    we_have: BitVec<u8, Msb0>,
    interested: bool,
    choked: bool,
}

impl State {
    fn new() -> Self {
        Self {
            they_have: BitVec::new(),
            we_have: BitVec::new(),
            interested: false,
            choked: false,
        }
    }
}

impl Actor for Peer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct Handshake {
    pub info_hash: [u8; 20],
    pub peer_id: [u8; 20],
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ConnectedMsg {
    tcp_stream: TcpStream,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct RecvMessage {
    msg: BittorrentMessage,
}

impl Handler<Handshake> for Peer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: Handshake, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received Handshake");

        let addr = ctx.address();
        let ip_address = self.ip_address.clone();
        let port = self.port;
        let info_hash = msg.info_hash.clone();
        let peer_id = msg.peer_id.clone();

        let fut: Pin<Box<dyn Future<Output = Result<TcpStream, NetworkError>> + Send>> =
            Box::pin(async move {
                debug!("Connecting to peer at {}:{}", ip_address, port);

                let mut tcp_stream = match TcpStream::connect((ip_address, port)).await {
                    Ok(tcp_stream) => tcp_stream,
                    Err(err) => {
                        error!("Connection error: {}", err);

                        return Err(err.into());
                    }
                };

                debug!("Connection established");

                let bittorrent_handshake = BittorrentHandshake { info_hash, peer_id };

                tcp_stream
                    .write_all(&bittorrent_handshake.serialize())
                    .await?;

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

                Ok(tcp_stream)
            });

        Box::pin(fut.into_actor(self).map(|tcp_stream_result, actor, ctx| {
            if let Ok(tcp_stream) = tcp_stream_result {
                let (read_half, write_half) = tcp_stream.into_split();
                let fut = read_loop(read_half, ctx.address());

                actor.write_half = Some(Rc::new(Mutex::new(write_half)));
                actor.read_loop = Some(ctx.spawn(fut.into_actor(actor)));
            }
        }))
    }
}

fn read_loop(mut read_half: OwnedReadHalf, addr: Addr<Peer>) -> impl Future<Output = ()> {
    let fut = Box::pin(async move {
        loop {
            let mut bytes = vec![0u8; 4];
            read_half.read_exact(&mut bytes).await.unwrap();
            let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
            bytes.resize(len + 4, 0u8);

            trace!("Reading message of length {}", len);

            match read_half.read_exact(&mut bytes[4..len]).await {
                Ok(b) => {
                    debug!("Read {} bytes", b);
                }
                Err(err) => {
                    error!("Read error: {}", err);
                    return;
                }
            }
            let msg = match BittorrentMessage::deserialize(&bytes) {
                Ok(m) => m,
                Err(err) => {
                    error!("Peer message error: {:?}", err);
                    return;
                }
            };

            trace!("Got message {:?}", msg);
            addr.do_send(RecvMessage { msg });

            return;
        }
    });

    fut
}

impl Handler<RecvMessage> for Peer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: RecvMessage, _ctx: &mut Context<Self>) -> Self::Result {
        let messages = handle_state(msg.msg, &mut self.state);
        let bytes: Vec<u8> = messages
            .into_iter()
            .map(|msg| msg.serialize())
            .flatten()
            .collect();

        if let Some(write_half) = self.write_half.clone() {
            Box::pin(
                async move {
                    write_half.lock().unwrap().write_all(&bytes);
                }
                .into_actor(self),
            )
        } else {
            Box::pin(
                async {
                    panic!("Somehow connection is not available. This is a program error");
                }
                .into_actor(self),
            )
        }
    }
}

fn handle_state(msg: BittorrentMessage, state: &mut State, filer: Addr<Filer>) -> Vec<BittorrentMessage> {
    match msg {
        BittorrentMessage::KeepAlive => {
            // Do nothing
        }
        BittorrentMessage::Choke => {
            state.choked = true;
        }
        BittorrentMessage::Unchoke => {
            state.choked = true;
        }
        BittorrentMessage::Interested => {
            state.interested = true;
        }
        BittorrentMessage::NotInterested => {
            state.interested = false;
        }
        BittorrentMessage::Have { piece_index } => {
            state.they_have.set(piece_index as usize, true);
        }
        BittorrentMessage::BitField { bitfield } => {
            state.they_have = BitVec::<_, Msb0>::from_slice(&bitfield);
        }
        BittorrentMessage::Request {
            index,
            begin,
            length,
        } => {
            if let Some(bit_ref) = state.they_have.get(index as usize) {
                if *bit_ref {
                    filer.send(PieceRequest { 
                        index,
                        begin,
                        length
                    }).into_actor(self);
                    // notify filer that we want to send a piece
                }
            }
        }
    }

    vec![]
}

/*#[derive(Message)]
#[rtype(result = "()")]
struct SendPiece {
    index: u32,
    begin: u32,
    block: Vec<u8>,
}*/

/*impl Handler<SendPiece> for Peer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: SendPiece, ctx: &mut Context<Self>) -> Self::Result {
        unimplemented!()
    }
}*/

#[derive(Debug)]
pub enum NetworkError {
    IOErr(io::Error),
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
    Have { piece_index: u32 },
    BitField { bitfield: Vec<u8> },
    Request { index: u32, begin: u32, length: u32 },
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
        unimplemented!()
        /*match self {
            BittorrentMessage::KeepAlive => vec![0, 0, 0, 0],
            BittorrentMessage::Choke => vec![0, 0, 0, 1, 0],
            BittorrentMessage::Unchoke => vec![0, 0, 0, 1, 1],
            BittorrentMessage::Interested => vec![0, 0, 0, 1, 2],
            BittorrentMessage::NotInterested => vec![0, 0, 0, 1, 3],
            BittorrentMessage::Have { piece_index } => {
                let mut v: Vec<u8> = Vec::with_capacity(4 + 1 + 4);
                v.extend_from_slice(&(1 + 4).to_be_bytes());
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
            BittorrentMessage::Request { index, begin, length } => {
                let mut v = Vec::with_capacity(4 + 1 + 12);
                v.extend_from_slice(&(1 + 12).to_be_bytes());
                v.push(6);
                v.extend_from_slice(&index.to_be_bytes());
                v.extend_from_slice(&begin.to_be_bytes());
                v.extend_from_slice(&length.to_be_bytes());
                v
            }
        }*/
    }
}
