use bitvec::{order::Msb0, vec::BitVec};

use crate::{metadata::Info, peer::BittorrentMessage};

pub struct PeerState {
    they_have: BitVec<u8, Msb0>,
    choking_them: bool,
    choking_us: bool,
    interested_them: bool,
    interested_us: bool,
    assigned_pieces: BitVec,
    pieces_received: usize,
}

impl PeerState {
    pub fn new(info: &Info) -> Self {
        Self {
            they_have: BitVec::repeat(false, info.pieces.len()),
            choking_them: false,
            choking_us: false,
            interested_them: false,
            interested_us: false,
            assigned_pieces: BitVec::repeat(false, info.pieces.len()),
            pieces_received: 0,
        }
    }

    pub fn new_message(&mut self, msg: &BittorrentMessage) {
        match msg {
            BittorrentMessage::KeepAlive => {
                // Do nothing
            }
            BittorrentMessage::Choke => {
                self.choking_us = true;
            }
            BittorrentMessage::Unchoke => {
                self.choking_us = false;
            }
            BittorrentMessage::Interested => {
                self.interested_them = true;
            }
            BittorrentMessage::NotInterested => {
                self.interested_us = false;
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

    pub fn pieces_we_can_request(&self, we_have: &BitVec<u8, Msb0>) -> Vec<u32> {
        let mut can_request_pieces = vec![];
        for i in 0..we_have.len() {
            if let (Some(we_have_bit_ref), Some(they_have_bit_ref)) = (we_have.get(i), self.they_have.get(i))
                && *we_have_bit_ref && *they_have_bit_ref {
                    can_request_pieces.push(i as u32);
                }
        }

        can_request_pieces
    }

    pub fn are_choking_them(&self) -> bool {
        self.choking_them
    }
}
