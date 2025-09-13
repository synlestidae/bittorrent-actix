use std::collections::HashSet;

use bitvec::{order::Msb0, vec::BitVec};

use crate::{metadata::{Info, PieceRequest}, piece_tracker::PieceTracker};

pub struct PeerState {
    pub handshake: bool,
    pub they_have: BitVec<u8, Msb0>,
    pub them_choking: bool,
    pub them_interested: bool,
    pub us_choking: bool,
    pub us_interested: bool,
    pub assigned_pieces: Vec<u32>,
    pub pieces_received: HashSet<u32>,
    pub cancels: Vec<PieceRequest>,
    pub they_want: Vec<u32>,
    pub piece_tracker: PieceTracker,
    pub piece_requests_sent: i32,
    pub piece_requests: Vec<PieceRequest>,
}

impl PeerState {
    pub fn new(info: Info) -> Self {
        Self {
            handshake: false,
            they_have: BitVec::new(),
            them_choking: false,
            them_interested: false,
            us_choking: true,
            us_interested: false,
            assigned_pieces: Vec::new(),
            pieces_received: HashSet::new(),
            cancels: Vec::new(),
            they_want: Vec::new(),
            piece_tracker: PieceTracker::new(info),
            piece_requests_sent: 0,
            piece_requests: Vec::new(),
        }
    }

    pub fn assign_pieces(&mut self, assigned_pieces: Vec<u32>) {
        self.assigned_pieces = assigned_pieces.into_iter().collect();
    }
}
