use std::collections::HashMap;
use std::collections::HashSet;
use std::net::Ipv4Addr;
use std::ops::Add;

use bitvec::vec::BitVec;
use log::error;
use rand::Rng;

use crate::bittorrent::BittorrentHandshake;
use crate::bittorrent::BittorrentMessage;
use crate::bittorrent::ExtensionMessage;
use crate::bittorrent::Info;
use crate::bittorrent::Magnet;
use crate::bittorrent::Piece;
use crate::bittorrent::PieceRequest;

use crate::commander::Command;
use crate::commander::connection_state::ConnectionState;
use crate::commander::metadata_state::MagnetMetadataTracker;
use crate::commander::metadata_state::MetadataState;
use crate::commander::piece_state::PieceState;

use crate::filer::WholePiece;

const MAX_PIECE_REQUESTS: usize = 20;

pub struct PeerState {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub connection_state: ConnectionState,
    pub metadata_state: Option<MetadataState>,
    pub piece_state: PieceState,
    pub outgoing_messages: Vec<BittorrentMessage>,
    pub dead: bool,
    pub our_peer_id: [u8; 20],
}

impl PeerState {
    pub fn new(address: (Ipv4Addr, u16)) -> Self {
        let (ip_address, port) = address;

        let mut rng = rand::rng();
        let peer_id = rng.random::<[u8; 20]>();

        Self {
            ip_address,
            port,
            connection_state: ConnectionState::new(),
            piece_state: PieceState::new(),
            metadata_state: None,
            outgoing_messages: vec![],
            dead: false,
            our_peer_id: peer_id,
        }
    }

    pub fn magnet(&mut self, magnet: Magnet) {
        match self.metadata_state {
            Some(MetadataState::HaveMetdata { .. }) => return,
            _ => {}
        }

        let magnet_metadata_tracker = MagnetMetadataTracker::new(magnet);

        self.metadata_state = Some(MetadataState::HaveMagnet {
            magnet: magnet_metadata_tracker,
        });
    }

    pub fn info(&mut self, info: Info) {
        self.metadata_state = Some(MetadataState::HaveMetdata { info });
    }

    pub fn can_assign_pieces(&self) -> bool {
        match self.metadata_state {
            None => false,
            Some(MetadataState::HaveMagnet { .. }) => false,
            _ => true,
        }
    }

    pub fn assign_piece(&mut self, piece_index: u32) {
        self.piece_state.assign_piece(piece_index);
    }

    pub fn set_piece_assignment(&mut self, assigned_pieces: Vec<u32>) {
        self.piece_state.assigned_pieces = assigned_pieces;
    }

    pub fn change(&mut self, change: PeerChange) {
        match change {
            PeerChange::RecvMessage { message } => self.apply_message(message),
            PeerChange::PieceComplete { piece_index } => self.apply_piece_complete(piece_index),
        }
    }

    fn apply_message(&mut self, msg: BittorrentMessage) {
        match msg {
            BittorrentMessage::Choke => self.connection_state.them_choking = true,
            BittorrentMessage::Unchoke => self.connection_state.them_choking = false,
            BittorrentMessage::Interested => self.connection_state.them_interested = true,
            BittorrentMessage::NotInterested => self.connection_state.them_interested = false,
            BittorrentMessage::Have { piece_index } => {
                let piece_set = self.piece_state.they_have(piece_index);
                if !piece_set {
                    error!("Peer exceeded maximum piece index");
                    self.dead = true;
                }
            }
            BittorrentMessage::Bitfield { bitfield } => {
                self.piece_state.they_have = BitVec::from_vec(bitfield);
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => {
                self.piece_state.piece_tracker.add_piece(Piece {
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
                self.piece_state
                    .piece_requests_from_them
                    .push(PieceRequest {
                        index,
                        begin,
                        length,
                    });
            }
            BittorrentMessage::Cancel { .. } => (),
            BittorrentMessage::Port { .. } => (),
            BittorrentMessage::Handshake { .. } => {
                self.connection_state.handshake_received = true;
            }
            BittorrentMessage::ExtensionMessage { message } => match message {
                ExtensionMessage::Request { piece_index: _ } => {
                    todo!(
                        "Here I need to return an extension message with the metadata slice they requested"
                    )
                }
                ExtensionMessage::Data {
                    piece_index,
                    total_size: _,
                    data,
                } => match self.metadata_state {
                    Some(MetadataState::HaveMagnet { ref mut magnet }) => {
                        magnet.add_piece(piece_index, data);
                    }
                    _ => {}
                },
                ExtensionMessage::Reject { piece_index: _ } => {
                    todo!(
                        "I've forgotten what reject does but I think it happens when the piece request is bad"
                    )
                }
            },
        }
    }

    fn apply_piece_complete(&mut self, piece_index: u32) {
        match self.piece_state.we_have.get_mut(piece_index as usize) {
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

    pub fn commands(&mut self, info_hash: [u8; 20]) -> Vec<Command> {
        let ip_address = self.ip_address;
        let port = self.port;

        let mut commands = Vec::new();

        if !self.connection_state.handshake_sent {
            let handshake = BittorrentHandshake::new(info_hash.clone(), self.our_peer_id);
            let handshake_message = BittorrentMessage::Handshake { handshake };

            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: handshake_message,
            });
        }

        if !self.connection_state.bitfield_sent {
            let bitfield = self.piece_state.we_have.clone().into_vec();
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Bitfield { bitfield },
            });
            self.connection_state.bitfield_sent = true;
        }

        if self.connection_state.us_choking {
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Unchoke,
            });
            self.connection_state.us_choking = false;
        }

        if !self.connection_state.us_interested {
            self.connection_state.us_interested = true;
            commands.push(Command::SendPeerMessage {
                ip_address,
                port,
                message: BittorrentMessage::Interested,
            });
        }

        // Commander needs to assign pieces
        // TODO make sure commander handles piece assignments

        // complete any pieces
        if let Some(MetadataState::HaveMetdata { info }) = &self.metadata_state {
            let complete_pieces = self.piece_state.piece_tracker.complete_pieces(&info);
            for complete_piece in complete_pieces.into_iter() {
                self.piece_state.piece_tracker.remove(complete_piece.index);

                if info.piece_correct(complete_piece.index, &complete_piece.data) {
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
            if !self.connection_state.them_choking {
                let piece_requests = self.piece_state.piece_tracker.next_pieces(
                    info,
                    &self.piece_state.assigned_pieces,
                    MAX_PIECE_REQUESTS,
                );

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
        }

        // handle pieces they request from us
        if !self.connection_state.us_choking {
            for request in self.piece_state.piece_requests_from_them.drain(..) {
                commands.push(Command::PieceRequestFromThem {
                    ip_address,
                    port,
                    request,
                });
            }
        } else {
            self.piece_state.piece_requests_from_them.retain(|_| false);
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

    pub fn completion_stats(&self, info: &Info) -> CompletionStats {
        let mut stats = HashMap::new();
        for piece_index in self.piece_state.assigned_pieces.iter() {
            if let Some(ref piece) = self
                .piece_state
                .piece_tracker
                .get_piece(&info, *piece_index)
            {
                let percentage = piece.len() as f32 / info.piece_length(*piece_index) as f32;
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
