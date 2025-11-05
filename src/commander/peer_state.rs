use std::{
    collections::{HashMap, HashSet},
    net::Ipv4Addr,
    ops::Add,
};

use super::piece_tracker::PieceTracker;
use crate::bittorrent::{Info, PieceRequest};
use crate::{
    bittorrent::{BittorrentMessage, Piece},
    commander::Command,
    filer::WholePiece,
};
use bitvec::{order::Msb0, vec::BitVec};

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
    outgoing_messages: Vec<BittorrentMessage>,
    piece_requests_from_them: Vec<PieceRequest>,
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
            bitfield_sent: false,
            assigned_pieces: Vec::new(),
            piece_tracker: PieceTracker::new(info),
            outgoing_messages: Vec::new(),
            piece_requests_from_them: Vec::new(),
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
        match change {
            PeerChange::RecvMessage { message } => self.apply_message(message),
            PeerChange::PieceComplete { piece_index } => self.apply_piece_complete(piece_index),
        }
    }

    fn apply_message(&mut self, msg: BittorrentMessage) {
        match msg {
            BittorrentMessage::Choke => self.them_choking = true,
            BittorrentMessage::Unchoke => self.them_choking = false,
            BittorrentMessage::Interested => self.them_interested = true,
            BittorrentMessage::NotInterested => self.them_interested = false,
            BittorrentMessage::Have { piece_index } => {
                match self.they_have.get_mut(piece_index as usize) {
                    Some(mut bit) => *bit = true,
                    None => (),
                }
            }
            BittorrentMessage::Bitfield { bitfield } => {
                self.they_have = BitVec::from_vec(bitfield);
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => {
                self.piece_tracker.add_piece(Piece {
                    index,
                    begin,
                    block,
                });
            }
            BittorrentMessage::KeepAlive => (),
            BittorrentMessage::Request {
                index,
                begin,
                length,
            } => {
                self.piece_requests_from_them.push(PieceRequest {
                    index,
                    begin,
                    length,
                });
            }
            BittorrentMessage::Cancel { .. } => (),
            BittorrentMessage::Port { .. } => (),
            _ => unimplemented!(),
        }
    }

    fn apply_piece_complete(&mut self, piece_index: u32) {
        match self.we_have.get_mut(piece_index as usize) {
            Some(mut have_piece) => {
                if !*have_piece {
                    self.outgoing_messages
                        .push(BittorrentMessage::Have { piece_index });
                }
                *have_piece = true;
            }
            None => (),
        }
    }

    pub fn commands(&mut self) -> Vec<Command> {
        let ip_address = self.ip_address;
        let port = self.port;

        let mut commands = Vec::new();

        if !self.bitfield_sent {
            let bitfield = self.we_have.clone().into_vec();
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Bitfield { bitfield },
            });
            self.bitfield_sent = true;
        }

        if self.us_choking {
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Unchoke,
            });
            self.us_choking = false;
        }

        if !self.us_interested {
            self.us_interested = true;
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Interested,
            });
        }

        // Commander needs to assign pieces
        // TODO make sure commander handles piece assignments

        // complete any pieces
        let complete_pieces = self.piece_tracker.complete_pieces();
        for complete_piece in complete_pieces.into_iter() {
            self.piece_tracker.remove(complete_piece.index);

            if self
                .info
                .piece_correct(complete_piece.index, &complete_piece.data)
            {
                commands.push(Command::WholePiece {
                    ip_address,
                    port,
                    whole_piece: WholePiece {
                        index: complete_piece.index,
                        data: complete_piece.data,
                    },
                });
            }
        }

        // request them for pieces
        if !self.them_choking {
            let piece_requests = self
                .piece_tracker
                .next_pieces(&self.assigned_pieces, MAX_PIECE_REQUESTS);

            for piece_request in piece_requests.into_iter() {
                commands.push(Command::SendPeerMessage {
                    ip_address,
                    port,
                    message: BittorrentMessage::Request {
                        index: piece_request.index,
                        begin: piece_request.begin,
                        length: piece_request.length,
                    },
                });
            }
        }

        // handle pieces they request from us
        if !self.us_choking {
            for request in self.piece_requests_from_them.drain(..) {
                commands.push(Command::PieceRequestFromThem {
                    ip_address,
                    port,
                    request,
                });
            }
        } else {
            self.piece_requests_from_them.retain(|_| false);
        }

        for message in self.outgoing_messages.drain(..) {
            commands.push(Command::SendPeerMessage {
                ip_address: self.ip_address,
                port: self.port,
                message,
            });
        }

        commands
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

#[derive(Debug)]
pub enum PeerChange {
    RecvMessage { message: BittorrentMessage },
    PieceComplete { piece_index: u32 },
}

/*#[cfg(test)]
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
        peer_state::{PeerChange, PeerState},
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
}*/
