use std::{collections::HashSet, net::Ipv4Addr};

use bitvec::{order::Msb0, vec::BitVec};
use log::{debug, trace};

use crate::{
    coordinator::PeerControlGlobal,
    filer::RequestPiece,
    metadata::{CompletePiece, Info, Piece, PieceRequest},
    peer::{
        BittorrentMessage, BittorrentMessageError, HavePiece, PeerControl, PeerControlEnvelope,
    },
    piece_tracker::PieceTracker,
};

const MAX_PIECE_REQUESTS: usize = 10;

pub struct PeerState {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub info: Info,
    pub handshake: bool,
    pub we_have: BitVec<u8, Msb0>,
    pub they_have: BitVec<u8, Msb0>,
    pub them_choking: bool,
    pub them_interested: bool,
    pub us_choking: bool,
    pub us_interested: bool,
    pub assigned_pieces: Vec<u32>,
    pub pieces_received_from_them: Vec<u32>,
    pub pieces_received_from_other_peers: HashSet<u32>,
    pub cancels: Vec<PieceRequest>,
    pub they_want: Vec<u32>,
    pub piece_tracker: PieceTracker,
    pub piece_requests_sent: i32,
    pub piece_requests: Vec<PieceRequest>,
    pub bitfield_sent: bool,
    pub new_pieces_received: Vec<u32>,
}

impl PeerState {
    pub fn new(info: Info, address: (Ipv4Addr, u16), we_have: BitVec<u8, Msb0>) -> Self {
        let (ip_address, port) = address;

        Self {
            ip_address,
            port,
            info: info.clone(),
            handshake: false,
            they_have: BitVec::new(),
            we_have,
            them_choking: false,
            them_interested: false,
            us_choking: true,
            us_interested: false,
            assigned_pieces: Vec::new(),
            pieces_received_from_other_peers: HashSet::new(),
            pieces_received_from_them: Vec::new(),
            cancels: Vec::new(),
            they_want: Vec::new(),
            piece_tracker: PieceTracker::new(info),
            piece_requests_sent: 0,
            piece_requests: Vec::new(),
            bitfield_sent: false,
            new_pieces_received: Vec::new(),
        }
    }

    pub fn set_piece_assignment(&mut self, assigned_pieces: Vec<u32>) {
        self.assigned_pieces = assigned_pieces.into_iter().collect();
    }

    pub fn change(&mut self, change: PeerChange) {
        if let Some(them_choking) = change.choking {
            self.them_choking = them_choking;
        }

        if let Some(them_interested) = change.interested {
            self.them_interested = them_interested;
        }

        if let Some(bitfield) = change.bitfield {
            // TODO check the bounds
            self.we_have = bitfield;
        }

        for piece_index in change.pieces_received_from_other_peers.into_iter() {
            if let Some(ref mut bit) = self.we_have.get_mut(piece_index as usize) {
                bit.set(true);
            }
        }

        for piece in change.pieces_received_from_them.into_iter() {
            self.piece_tracker.add_piece(piece);
        }

        for piece_index in change.pieces_requested {
            self.piece_requests.push(piece_index);
        }
    }

    pub fn peer_command(&mut self) -> PeerCommand {
        let ip_address = self.ip_address;
        let port = self.port;

        let mut peer_control = Vec::new();
        let mut piece_requests_to_us = Vec::new();
        let mut peer_control_global = Vec::new();

        if !self.bitfield_sent {
            trace!(
                "Sending bitfield to {}:{} {}",
                ip_address, port, self.we_have
            );
            peer_control.push(PeerControl::Bitfield {
                bitfield: self.we_have.clone(),
            });
            self.bitfield_sent = true;
        }

        if self.us_choking {
            trace!("Unchoking {}:{}", ip_address, port);
            peer_control.push(PeerControl::UnchokeThem);
            self.us_choking = false;
        }

        if !self.us_interested {
            trace!("Telling {}:{} we're interested", ip_address, port);
            peer_control.push(PeerControl::Interested);
            self.us_interested = true;
        }

        if self.assigned_pieces.is_empty() {
            let len = self.info.num_pieces();
            self.we_have.resize(len, false);
            self.they_have.resize(len, false);

            let assigned_pieces: Vec<u32> = self.they_have.iter_ones().map(|i| i as u32).collect();

            if assigned_pieces.len() > 0 {
                trace!("Assigning pieces to download from {}:{}", ip_address, port);
                if !assigned_pieces.is_empty() {
                    self.set_piece_assignment(assigned_pieces.to_vec());
                    peer_control.push(PeerControl::AssignPieces { assigned_pieces });
                }
            }
        }

        // TODO this should only send Have if they dont have a "have" already
        // Trick to doing this is to go through all the peers after this loop
        // Drain all the have messages
        // And then iterate over the peers again and send them if a bit on we_have was flipped
        for piece_index in self.new_pieces_received.drain(..) {
            trace!(
                "We received piece {} from {}:{}",
                piece_index, ip_address, port
            );
        }

        // complete any pieces
        let complete_pieces = self.piece_tracker.complete_pieces();
        let mut correct_pieces = Vec::new();
        if complete_pieces.len() > 0 {
            debug!("{} complete piece(s)", complete_pieces.len());
        }
        for complete_piece in complete_pieces.into_iter() {
            debug!("Piece {} is complete", complete_piece.index);
            self.piece_tracker.mark_complete(complete_piece.index);
            self.piece_tracker.remove(complete_piece.index);

            if self
                .info
                .piece_correct(complete_piece.index, &complete_piece.data)
            {
                trace!("Piece {} is correct", complete_piece.index);
                correct_pieces.push(complete_piece);
            } else {
                trace!("Piece {} is incorrect", complete_piece.index);
            }
        }
        correct_pieces.iter().for_each(|piece| {
            peer_control_global.push(PeerControlGlobal::HavePiece {
                piece_index: piece.index,
            });
        });

        for piece_index in self.pieces_received_from_them.drain(..) {
            peer_control.push(PeerControl::HavePiece { index: piece_index });
        }

        // get next piece requests to them
        let piece_requests = self
            .piece_tracker
            .next_pieces(&self.assigned_pieces, MAX_PIECE_REQUESTS);
        if !piece_requests.is_empty() {
            peer_control.push(PeerControl::RequestPieces { piece_requests });
        }

        // piece requests to US
        for piece_request in self.piece_requests.drain(..) {
            piece_requests_to_us.push(RequestPiece {
                ip_address: ip_address,
                port: port,
                index: piece_request.index,
                begin: piece_request.begin,
                length: piece_request.length,
            });
        }

        PeerCommand {
            peer_control,
            peer_control_global,
            complete_pieces: correct_pieces,
            piece_requests_to_us,
        }
    }
}

#[derive(Debug, Default)]
pub struct PeerChange {
    pub choking: Option<bool>,
    pub interested: Option<bool>,
    pub bitfield: Option<BitVec<u8, Msb0>>,
    pub pieces_received_from_other_peers: HashSet<u32>,
    pub pieces_received_from_them: Vec<Piece>,
    pub pieces_requested: Vec<PieceRequest>,
    pub pieces_have: Vec<u32>,
}

impl PeerChange {
    pub fn handle_message(&mut self, msg: BittorrentMessage) {
        match msg {
            BittorrentMessage::Choke => self.choking = Some(true),
            BittorrentMessage::Unchoke => self.choking = Some(false),
            BittorrentMessage::Interested => self.interested = Some(true),
            BittorrentMessage::NotInterested => self.interested = Some(false),
            BittorrentMessage::Have { piece_index } => self.pieces_have.push(piece_index),
            BittorrentMessage::Bitfield { bitfield } => {
                self.bitfield = Some(BitVec::from_vec(bitfield))
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => self.pieces_received_from_them.push(Piece {
                index,
                begin,
                block,
            }),
            BittorrentMessage::KeepAlive => (),
            BittorrentMessage::Request {
                index,
                begin,
                length,
            } => self.pieces_requested.push(PieceRequest {
                index,
                begin,
                length,
            }),
            BittorrentMessage::Cancel { .. } => (),
            BittorrentMessage::Port { .. } => (),
        }
    }
}

pub struct PeerCommand {
    pub peer_control: Vec<PeerControl>,
    pub peer_control_global: Vec<PeerControlGlobal>,
    pub complete_pieces: Vec<CompletePiece>,
    pub piece_requests_to_us: Vec<RequestPiece>,
}
