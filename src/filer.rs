use actix::prelude::*;
use log::debug;

use std::net::Ipv4Addr;
use std::{collections::BTreeMap, sync::Arc};

use crate::commander::CommandSet;
use crate::coordinator::PeerControlGlobal;
use crate::peer::{PeerControl, PeerControlEnvelope};
use crate::{coordinator::Coordinator, metadata::Info};

pub struct Filer {
    info: Info,
    pieces: BTreeMap<u32, Arc<[u8]>>,
    coordinator_addr: Addr<Coordinator>,
}

impl Filer {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>) -> Self {
        Self {
            info,
            pieces: BTreeMap::new(),
            coordinator_addr,
        }
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
#[rtype(result = "()")]
pub struct WholePiece {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub index: u32,
    pub data: Arc<[u8]>,
}

impl Handler<WholePiece> for Filer {
    type Result = ();

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);

        self.pieces.insert(msg.index, msg.data);

        self.coordinator_addr.do_send(CommandSet {
            peer_control: vec![],
            peer_control_global: vec![PeerControlGlobal::HavePiece {
                piece_index: msg.index,
            }],
            piece_requests_to_us: vec![],
            complete_pieces: vec![],
            peer_change: vec![],
        });

        // TODO take the piece, arrange it, write it to disk where necessary
        // this includes creating the directory, files, etc
    }
}

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct RequestPieceEnvelope {
    pub request_pieces: Vec<RequestPiece>,
}

#[derive(Debug)]
pub struct RequestPiece {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}

impl Handler<RequestPieceEnvelope> for Filer {
    type Result = ();

    fn handle(&mut self, msg: RequestPieceEnvelope, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received {:?}", msg);
        for request_piece in msg.request_pieces.into_iter() {
            let piece = match self.pieces.get(&request_piece.index) {
                Some(p) => p,
                None => return,
            };

            let end_index = (request_piece.begin + request_piece.length) as usize;
            if end_index > piece.len() {
                // invalid request, ignore
                return;
            }

            let block = piece[request_piece.begin as usize
                ..(request_piece.begin + request_piece.length) as usize]
                .iter()
                .cloned()
                .collect();

            self.coordinator_addr.do_send(CommandSet {
                peer_control: vec![PeerControlEnvelope {
                    ip_address: request_piece.ip_address,
                    port: request_piece.port,
                    messages: vec![PeerControl::UploadPieceToThem {
                        index: request_piece.index,
                        begin: request_piece.begin,
                        block: block,
                    }],
                }], // TODO
                peer_control_global: vec![],
                piece_requests_to_us: vec![],
                complete_pieces: vec![],
                peer_change: vec![],
            });
        }
    }
}
