use bitvec::order::Msb0;
use bitvec::vec::BitVec;

use crate::bittorrent::PieceRequest;
use crate::commander::piece_tracker::PieceTracker;
use std::cmp::min;

pub struct PieceState {
    pub we_have: BitVec<u8, Msb0>,
    pub they_have: BitVec<u8, Msb0>,
    pub assigned_pieces: Vec<u32>,
    pub piece_tracker: PieceTracker,
    pub piece_requests_from_them: Vec<PieceRequest>,
}

const THEY_HAVE_MAX_BITS: usize = 800_000_000;

impl PieceState {
    pub fn new() -> Self {
        Self {
            we_have: BitVec::new(),
            they_have: BitVec::new(),
            assigned_pieces: vec![],
            piece_tracker: PieceTracker::new(),
            piece_requests_from_them: Vec::new(),
        }
    }
}

impl PieceState {
    pub fn assign_piece(&mut self, piece_index: u32) {
        if !self.assigned_pieces.contains(&piece_index) {
            self.assigned_pieces.push(piece_index);
        }
    }

    pub fn they_have(&mut self, piece_index: u32) -> bool {
        if piece_index as usize >= THEY_HAVE_MAX_BITS {
            return false;
        }
        let new_size = min(THEY_HAVE_MAX_BITS, piece_index as usize);
        self.they_have.resize(new_size, false);

        if let Some(ref mut bit_ref) = self.they_have.get_mut(piece_index as usize) {
            bit_ref.set(true);
            true
        } else {
            false
        }
    }
}
