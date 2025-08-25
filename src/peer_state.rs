use bitvec::{order::Msb0, vec::BitVec};

use crate::{
    metadata::{Info, PieceRequest},
    peer::BittorrentMessage,
};

pub struct PeerState {
    pub handshake: bool,
    pub they_have: BitVec<u8, Msb0>,
    pub them_choking: bool,
    pub them_interested: bool,
    pub us_choking: bool,
    pub us_interested: bool,
    pub assigned_pieces: Vec<u32>,
    pub pieces_received: usize,
    pub cancels: Vec<PieceRequest>,
}

impl PeerState {
    pub fn new(info: &Info) -> Self {
        Self {
            handshake: false,
            they_have: BitVec::repeat(false, info.pieces.len()),
            them_choking: false,
            them_interested: false,
            us_choking: false,
            us_interested: false,
            assigned_pieces: Vec::new(),
            pieces_received: 0,
            cancels: Vec::new(),
        }
    }

    pub fn new_message(&mut self, msg: &BittorrentMessage) {
        match msg {
            BittorrentMessage::KeepAlive => {
                // Do nothing
            }
            BittorrentMessage::Choke => {
                self.them_choking = true;
            }
            BittorrentMessage::Unchoke => {
                self.them_choking = false;
            }
            BittorrentMessage::Interested => {
                self.them_interested = true;
            }
            BittorrentMessage::NotInterested => {
                self.them_interested = false;
            }
            BittorrentMessage::Have { piece_index } => {
                self.they_have.set(*piece_index as usize, true);
            }
            BittorrentMessage::BitField { bitfield } => {
                self.they_have = BitVec::<_, Msb0>::from_slice(bitfield);
            }
            _ => (),
        };
    }

    pub fn assigned_pieces(&self) -> Vec<u32> {
        self.assigned_pieces.clone()
    }

    pub fn assign_pieces(&mut self, assigned_pieces: Vec<u32>) {
        self.assigned_pieces = assigned_pieces;
    }
}
