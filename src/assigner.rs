use bitvec::{order::Msb0, vec::BitVec};
use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
};

use crate::peer_state::PeerState;

pub struct Assigner<'a> {
    pub we_have: &'a BitVec<u8, Msb0>,
    pub peers: &'a mut HashMap<(Ipv4Addr, u16), PeerState>,
}

impl<'a> Assigner<'a> {
    pub fn assign(&mut self) {
        let mut we_need: HashSet<u32> = self.we_have.iter_zeros().map(|i| i as u32).collect();
        let pieces_per_peer = we_need.len() / we_need.len();

        for peer_state in self.peers.values_mut() {
            let completion_stats = peer_state.completion_stats();
            let pieces = completion_stats
                .above_threshold(0.10)
                .into_iter()
                .chain(we_need.drain().take(pieces_per_peer))
                .collect();
            peer_state.set_piece_assignment(pieces);
        }

        let mut remaining_pieces: Vec<u32> = we_need.drain().collect();
        remaining_pieces.reverse();
        let peer_keys: Vec<(Ipv4Addr, u16)> = self.peers.keys().cloned().collect();

        for (ip_addr, port) in peer_keys.iter().cycle() {
            let peer_state = self.peers.get_mut(&(*ip_addr, *port)).unwrap();
            match remaining_pieces.pop() {
                Some(piece_index) => peer_state.assign_piece(piece_index),
                None => return,
            }
        }
    }
}
