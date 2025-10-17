use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    ops::Add,
};

use bitvec::{order::Msb0, vec::BitVec};
use log::{debug, trace};

use crate::{
    coordinator::PeerControlGlobal,
    filer::RequestPiece,
    metadata::{CompletePiece, Info, Piece, PieceRequest},
    peer::{BittorrentMessage, PeerControl},
    piece_tracker::PieceTracker,
};

const MAX_PIECE_REQUESTS: usize = 20;

pub struct PeerState {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    info: Info,
    bitfield_sent: bool,
    we_have: BitVec<u8, Msb0>,
    they_have: BitVec<u8, Msb0>,
    them_choking: bool,
    them_interested: bool,
    us_choking: bool,
    us_interested: bool,
    assigned_pieces: Vec<u32>,
    piece_tracker: PieceTracker,
    piece_requests_from_them: Vec<PieceRequest>,
    piece_haves_from_them: Vec<u32>,
    haves_to_send: Vec<u32>,
    changes: Vec<PeerChange>,
}

impl PeerState {
    pub fn new(info: Info, address: (Ipv4Addr, u16), we_have: BitVec<u8, Msb0>) -> Self {
        let (ip_address, port) = address;

        Self {
            ip_address,
            port,
            info: info.clone(),
            they_have: BitVec::new(),
            we_have,
            them_choking: false,
            them_interested: false,
            us_choking: true,
            us_interested: false,
            assigned_pieces: Vec::new(),
            piece_tracker: PieceTracker::new(info),
            bitfield_sent: false,
            piece_requests_from_them: Vec::new(),
            piece_haves_from_them: Vec::new(),
            haves_to_send: Vec::new(),
            changes: Vec::new(),
        }
    }

    pub fn assign_piece(&mut self, piece_index: u32) {
        if self.assigned_pieces.contains(&piece_index) {
            return;
        }
        self.assigned_pieces.push(piece_index);
    }

    pub fn set_piece_assignment(&mut self, assigned_pieces: Vec<u32>) {
        self.assigned_pieces = assigned_pieces.into_iter().collect();
    }

    pub fn change(&mut self, change: PeerChange) {
        self.changes.push(change);
    }

    fn apply_changes(&mut self) {
        for change in self.changes.drain(..).into_iter() {
            if let Some(bitfield) = change.bitfield {
                // TODO check the bounds
                self.they_have = bitfield;
            }

            if let Some(them_choking) = change.choking {
                self.them_choking = them_choking;
            }

            if let Some(them_interested) = change.interested {
                self.them_interested = them_interested;
            }

            for piece_index in change.pieces_completed_from_other_peers.into_iter() {
                // Hate the BitVec crate
                let idx = piece_index as usize;
                if idx < self.we_have.len() {
                    if !self.we_have[idx] {
                        self.haves_to_send.push(piece_index);
                    }
                    self.we_have.get_mut(idx).map(|mut b| b.set(true));
                }
            }

            for piece in change.pieces_received_from_them.into_iter() {
                self.piece_tracker.add_piece(piece);
            }

            for piece_index in change.pieces_requested {
                self.piece_requests_from_them.push(piece_index);
            }

            for piece_index in change.pieces_they_have {
                self.piece_haves_from_them.push(piece_index);
            }
        }
    }

    pub fn peer_command(&mut self) -> PeerCommand {
        self.apply_changes();

        let ip_address = self.ip_address;
        let port = self.port;

        let mut peer_control = Vec::new();
        let mut piece_requests_to_us = Vec::new();
        let mut peer_control_global = Vec::new();

        if !self.bitfield_sent {
            let bitfield = self.we_have.clone().into_vec();
            trace!("Sending bitfield to {}:{} {:?}", ip_address, port, bitfield,);
            peer_control.push(PeerControl::Bitfield { bitfield });
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

            if !assigned_pieces.is_empty() {
                trace!("Assigning pieces to download from {}:{}", ip_address, port);
                if !assigned_pieces.is_empty() {
                    self.set_piece_assignment(assigned_pieces.to_vec());
                }
            }
        }

        // complete any pieces
        let complete_pieces = self.piece_tracker.complete_pieces();
        let mut correct_pieces = Vec::new();
        if !complete_pieces.is_empty() {
            debug!("{} complete piece(s)", complete_pieces.len());
        }
        for complete_piece in complete_pieces.into_iter() {
            debug!("Piece {} is complete", complete_piece.index);
            self.piece_tracker.remove(complete_piece.index);

            if self
                .info
                .piece_correct(complete_piece.index, &complete_piece.data)
            {
                trace!("Piece {} is correct", complete_piece.index);
                self.piece_tracker.mark_complete(complete_piece.index);
                self.haves_to_send.push(complete_piece.index);
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

        // send Have for every piece that another peer gave us
        for index in self.haves_to_send.drain(..) {
            peer_control.push(PeerControl::HavePiece { index });
        }

        // get next piece requests to them
        if !self.them_choking {
            let piece_requests = self
                .piece_tracker
                .next_pieces(&self.assigned_pieces, MAX_PIECE_REQUESTS);
            if !piece_requests.is_empty() {
                peer_control.push(PeerControl::RequestPieces { piece_requests });
            }
        }

        // piece requests to US
        if !self.us_choking {
            for piece_request in self.piece_requests_from_them.drain(..) {
                piece_requests_to_us.push(RequestPiece {
                    ip_address,
                    port,
                    index: piece_request.index,
                    begin: piece_request.begin,
                    length: piece_request.length,
                });
            }
        }

        PeerCommand {
            peer_control,
            peer_control_global,
            complete_pieces: correct_pieces,
            piece_requests_to_us,
        }
    }

    pub fn completion_stats(&self) -> CompletionStats {
        let mut stats = HashMap::new();
        for piece_index in self.assigned_pieces.iter() {
            if let Some(ref piece) = self.piece_tracker.get_piece(*piece_index) {
                let percentage = piece.len() as f32 / self.info.piece_length(*piece_index) as f32;
                stats.insert((self.ip_address, self.port, *piece_index), percentage);
            }
        }

        CompletionStats { stats }
    }
}

pub struct CompletionStats {
    stats: HashMap<(Ipv4Addr, u16, u32), f32>,
}

impl CompletionStats {
    pub fn new() -> Self {
        Self {
            stats: HashMap::new(),
        }
    }

    pub fn above_threshold(&self, threshold: f32) -> HashSet<u32> {
        self.stats
            .iter()
            .filter(|((_, _, _), p)| **p > threshold)
            .map(|((_, _, i), _)| *i)
            .collect()
    }
}

impl Add for CompletionStats {
    type Output = CompletionStats;

    fn add(mut self, other: CompletionStats) -> CompletionStats {
        for (k, v) in other.stats.into_iter() {
            self.stats.insert(k, v);
        }

        self
    }
}

#[derive(Debug, Default)]
pub struct PeerChange {
    pub choking: Option<bool>,
    pub interested: Option<bool>,
    pub bitfield: Option<BitVec<u8, Msb0>>,
    pub pieces_completed_from_other_peers: HashSet<u32>,
    pub pieces_received_from_them: Vec<Piece>,
    pub pieces_they_have: Vec<u32>,
    pub pieces_requested: Vec<PieceRequest>,
}

impl PeerChange {
    pub fn handle_message(&mut self, msg: BittorrentMessage) {
        match msg {
            BittorrentMessage::Choke => self.choking = Some(true),
            BittorrentMessage::Unchoke => self.choking = Some(false),
            BittorrentMessage::Interested => self.interested = Some(true),
            BittorrentMessage::NotInterested => self.interested = Some(false),
            BittorrentMessage::Have { piece_index } => self.pieces_they_have.push(piece_index),
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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PeerCommand {
    pub peer_control: Vec<PeerControl>,
    pub peer_control_global: Vec<PeerControlGlobal>,
    pub complete_pieces: Vec<CompletePiece>,
    pub piece_requests_to_us: Vec<RequestPiece>,
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use bitvec::{order::Msb0, vec::BitVec};
    use ctor::ctor;
    use env_logger::init as init_env_logger;
    use sha1::{Digest, Sha1};

    use crate::{
        bencoding::Position,
        coordinator::PeerControlGlobal,
        metadata::{CompletePiece, Info, ModeFile, PieceRequest},
        peer::{BittorrentMessage, PeerControl},
        peer_state::{PeerChange, PeerCommand, PeerState},
    };

    #[ctor]
    fn init() {
        init_env_logger();
    }

    fn info(num_pieces: u32) -> Info {
        Info {
            pieces_length: 10,
            pieces: vec![vec![
                150, 148, 196, 235, 214, 115, 165, 226, 253, 38, 228, 178, 230, 79, 146, 233, 20,
                235, 217, 95,
            ]]
            .into_iter()
            .cycle()
            .take(num_pieces as usize)
            .collect(),
            file_info: ModeFile::Single { length: 10 },
            name: String::from("test.txt").into_bytes(),
            position: Position {
                offset: 0,
                length: 10,
            },
        }
    }

    #[test]
    pub fn test_peer_change_choke() {
        let mut peer_state = PeerState::new(
            info(1),
            ("10.0.0.1".parse().unwrap(), 1234),
            BitVec::from_vec(vec![0b1]),
        );
        let mut peer_change: PeerChange = Default::default();
        peer_change.handle_message(BittorrentMessage::Choke);

        peer_state.change(peer_change);

        assert_eq!(
            peer_state.peer_command(),
            PeerCommand {
                peer_control: vec![
                    PeerControl::Bitfield {
                        bitfield: vec![0b1],
                    },
                    PeerControl::UnchokeThem,
                    PeerControl::Interested
                ],
                ..Default::default()
            }
        );
    }

    #[test]
    pub fn test_peer_change_assign_unchoke() {
        let mut peer_state = PeerState::new(
            info(1),
            ("10.0.0.1".parse().unwrap(), 1234),
            BitVec::from_vec(vec![0b00000000]),
        );
        let mut peer_change: PeerChange = Default::default();
        peer_change.interested = Some(true);
        peer_change.handle_message(BittorrentMessage::Bitfield {
            bitfield: vec![0b10000000],
        });
        peer_change.handle_message(BittorrentMessage::Unchoke);

        peer_state.change(peer_change);

        assert_eq!(
            peer_state.peer_command(),
            PeerCommand {
                peer_control: vec![
                    PeerControl::Bitfield {
                        bitfield: vec![0b0],
                    },
                    PeerControl::UnchokeThem,
                    PeerControl::Interested,
                    PeerControl::RequestPieces {
                        piece_requests: vec![PieceRequest {
                            index: 0,
                            begin: 0,
                            length: 10,
                        }]
                    }
                ],
                ..Default::default()
            }
        );
    }

    #[test]
    pub fn test_peer_change_request_four_sub_pieces() {
        let mut peer_state = PeerState::new(
            info(8),
            ("10.0.0.1".parse().unwrap(), 1234),
            BitVec::from_vec(vec![0b00000000]),
        );
        let mut peer_change: PeerChange = Default::default();
        peer_change.interested = Some(true);
        peer_change.handle_message(BittorrentMessage::Bitfield {
            bitfield: vec![0b01010101],
        });
        peer_change.handle_message(BittorrentMessage::Unchoke);

        peer_state.change(peer_change);

        assert_eq!(
            peer_state.peer_command(),
            PeerCommand {
                peer_control: vec![
                    PeerControl::Bitfield {
                        bitfield: vec![0b0],
                    },
                    PeerControl::UnchokeThem,
                    PeerControl::Interested,
                    PeerControl::RequestPieces {
                        piece_requests: vec![
                            PieceRequest {
                                index: 1,
                                begin: 0,
                                length: 10,
                            },
                            PieceRequest {
                                index: 3,
                                begin: 0,
                                length: 10,
                            },
                            PieceRequest {
                                index: 5,
                                begin: 0,
                                length: 10,
                            },
                            PieceRequest {
                                index: 7,
                                begin: 0,
                                length: 10,
                            }
                        ],
                    }
                ],
                ..Default::default()
            }
        );
    }

    #[test]
    pub fn test_peer_change_request_first_second_and_third_piece() {
        let mut peer_state = PeerState::new(
            info(3),
            ("10.0.0.1".parse().unwrap(), 1234),
            BitVec::from_vec(vec![0b00000000]),
        );
        let mut first_peer_change: PeerChange = Default::default();
        first_peer_change.interested = Some(true);
        first_peer_change.handle_message(BittorrentMessage::Bitfield {
            bitfield: vec![0b11100000],
        });
        first_peer_change.handle_message(BittorrentMessage::Unchoke);

        peer_state.change(first_peer_change);

        let mut second_peer_change: PeerChange = Default::default();
        second_peer_change.handle_message(BittorrentMessage::Piece {
            index: 0,
            begin: 0,
            block: vec![0].into_iter().cycle().take(10).collect(),
        });
        peer_state.change(second_peer_change);
        let first_command = peer_state.peer_command();

        assert_eq!(
            first_command,
            PeerCommand {
                peer_control: vec![
                    PeerControl::Bitfield {
                        bitfield: vec![0x00]
                    },
                    PeerControl::UnchokeThem,
                    PeerControl::Interested,
                    PeerControl::HavePiece { index: 0 },
                    PeerControl::RequestPieces {
                        piece_requests: vec![
                            PieceRequest {
                                index: 1,
                                begin: 0,
                                length: 10,
                            },
                            PieceRequest {
                                index: 2,
                                begin: 0,
                                length: 10,
                            },
                        ],
                    }
                ],
                complete_pieces: vec![CompletePiece {
                    index: 0,
                    data: Arc::from(vec![0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                }],
                peer_control_global: vec![PeerControlGlobal::HavePiece { piece_index: 0 }],
                piece_requests_to_us: vec![]
            }
        );
    }

    #[test]
    pub fn test_peer_change_interested() {
        let mut peer_state =
            PeerState::new(info(1), ("10.0.0.1".parse().unwrap(), 1234), BitVec::new());
        let mut peer_change: PeerChange = Default::default();

        peer_change.handle_message(BittorrentMessage::Interested);
        peer_state.change(peer_change);

        //assert!(peer_state.them_interested);
    }

    #[test]
    pub fn test_peer_change_not_interested() {
        let mut peer_state =
            PeerState::new(info(1), ("10.0.0.1".parse().unwrap(), 1234), BitVec::new());
        let mut peer_change: PeerChange = Default::default();

        peer_change.handle_message(BittorrentMessage::NotInterested);
        peer_state.change(peer_change);

        //assert!(!peer_state.them_interested);
    }

    #[test]
    pub fn test_peer_change_have_piece() {
        let mut peer_state =
            PeerState::new(info(1), ("10.0.0.1".parse().unwrap(), 1234), BitVec::new());
        let mut peer_change: PeerChange = Default::default();

        peer_change.handle_message(BittorrentMessage::Have { piece_index: 0 });
        peer_state.change(peer_change);

        //assert_eq!(peer_state.pieces_they_have, vec![0])
    }

    #[test]
    pub fn test_peer_change_bitfield() {
        let mut peer_state =
            PeerState::new(info(1), ("10.0.0.1".parse().unwrap(), 1234), BitVec::new());
        let mut peer_change: PeerChange = Default::default();

        peer_change.handle_message(BittorrentMessage::Bitfield {
            bitfield: vec![0b11111111],
        });
        peer_state.change(peer_change);

        //assert_eq!(peer_state.they_have.into_vec(), bitvec.into_vec());
    }

    #[test]
    pub fn test_peer_request_piece() {
        let mut peer_state =
            PeerState::new(info(1), ("10.0.0.1".parse().unwrap(), 1234), BitVec::new());
        let mut peer_change: PeerChange = Default::default();

        peer_change.handle_message(BittorrentMessage::Request {
            index: 0,
            begin: 0,
            length: 10,
        });
        peer_state.change(peer_change);

        /*assert_eq!(
            peer_state.piece_requests,
            vec![PieceRequest {
                index: 0,
                begin: 0,
                length: 10
            }]
        );*/
    }
}
