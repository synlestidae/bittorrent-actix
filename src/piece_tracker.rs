use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use log::trace;

use crate::metadata::{CompletePiece, Info, Piece, PieceRequest};

#[derive(Debug)]
pub struct PieceTracker {
    pieces: HashMap<u32, Vec<Piece>>,
    requested_pieces: Vec<PieceRequest>,
    complete_pieces: HashSet<u32>,
    info: Info,
}

pub const PIECE_REQUEST_SIZE: u32 = 16_384;

impl PieceTracker {
    pub fn new(info: Info) -> Self {
        Self {
            pieces: HashMap::new(),
            requested_pieces: vec![],
            complete_pieces: HashSet::new(),
            info,
        }
    }

    pub fn add_piece(&mut self, mut piece: Piece) {
        self.requested_pieces.retain(|piece_request| {
            if piece_request.index != piece.index {
                return true;
            }

            let piece_begin = piece.begin;
            let piece_end = piece.begin + piece.block.len() as u32;

            let request_begin = piece_request.begin;
            let request_end = piece_request.begin + piece_request.length;

            if piece_begin <= request_begin && request_end <= piece_end {
                // Fully covered
                return false;
            }

            if piece_begin < request_end && piece_end > request_begin {
                // Any overlap
                return false;
            }

            true
        });

        if let Some(pieces) = self.pieces.get_mut(&piece.index) {
            if pieces.is_empty() {
                // shouldn't happen but whatever
                pieces.push(piece);
            } else {
                let last_piece_idx = pieces.len() - 1;
                let last_piece = &mut pieces[last_piece_idx];
                if last_piece.begin + last_piece.block.len() as u32 == piece.begin {
                    last_piece.block.append(&mut piece.block);
                } else {
                    pieces.push(piece);
                }

                pieces.sort_by_key(|p| p.begin);
            }
            return;
        }

        self.pieces.insert(piece.index, vec![piece]);
    }

    pub fn remove(&mut self, piece_index: u32) {
        self.pieces.remove(&piece_index);
    }

    pub fn is_complete(&self, piece_index: u32) -> bool {
        let pieces: &Vec<_> = match self.pieces.get(&piece_index) {
            Some(p) => p,
            None => return false,
        };

        let mut piece = &pieces[0];
        for i in 1..pieces.len() {
            let next_piece = &pieces[i];
            if piece.begin + piece.block.len() as u32 > next_piece.begin {
                return false;
            }
            piece = next_piece;
        }

        let complete =
            piece.begin + piece.block.len() as u32 >= self.info.piece_length(piece_index);
        trace!(
            "Length for {} is {} and needs to be {}",
            piece_index,
            piece.begin + piece.block.len() as u32,
            self.info.piece_length(piece_index)
        );
        complete
    }

    pub fn mark_complete(&mut self, piece_index: u32) {
        self.complete_pieces.insert(piece_index);
    }

    pub fn get_piece(&self, piece_index: u32) -> Option<Vec<u8>> {
        let pieces = self.pieces.get(&piece_index)?;
        if pieces.len() == 1 {
            return Some(pieces[0].block.clone());
        }

        let mut piece = vec![0u8; self.info.piece_length(piece_index) as usize];
        for sub_piece in pieces.iter() {
            let mut i = 0;
            let sub_piece_len = sub_piece.block.len() as u32;
            while i < sub_piece_len && sub_piece.begin + i < piece.len() as u32 {
                piece[(sub_piece.begin + i) as usize] = sub_piece.block[i as usize];
                i += 1;
            }
        }

        Some(piece)
    }

    pub fn next_pieces(&mut self, assigned_piece_indexes: &[u32], max: usize) -> Vec<PieceRequest> {
        if self.requested_pieces.len() >= max {
            return vec![];
        }
        let max_next = max - self.requested_pieces.len();

        let pieces = self
            .pieces
            .values()
            .flatten()
            .map(|p| PieceRequest {
                index: p.index,
                begin: p.begin,
                length: p.block.len() as u32,
            })
            .chain(self.requested_pieces.iter().cloned()) // so don't re-request piece
            .collect();

        let piece_iterator = PieceIterator::new(
            &pieces,
            assigned_piece_indexes,
            &self.complete_pieces,
            &self.info,
            PIECE_REQUEST_SIZE,
        );

        let new_pieces: Vec<_> = piece_iterator.take(max_next as usize).collect();
        self.requested_pieces.append(&mut new_pieces.clone());

        new_pieces
    }

    pub fn complete_pieces(&mut self) -> Vec<CompletePiece> {
        let mut complete_pieces = Vec::new();
        for piece_index in self.pieces.keys() {
            if self.is_complete(*piece_index) {
                let complete_piece = self.get_piece(*piece_index).unwrap();
                complete_pieces.push(CompletePiece {
                    index: *piece_index,
                    data: Arc::from(complete_piece),
                });
            }
        }

        complete_pieces
    }
}

struct PieceIterator<'a> {
    pieces: &'a Vec<PieceRequest>,
    new_pieces: Vec<PieceRequest>,
    assigned_pieces: &'a [u32],
    complete_pieces: &'a HashSet<u32>,
    info: &'a Info,
    request_size: u32,
}

impl<'a> PieceIterator<'a> {
    fn new(
        pieces: &'a Vec<PieceRequest>,
        assigned_pieces: &'a [u32],
        complete_pieces: &'a HashSet<u32>,
        info: &'a Info,
        request_size: u32,
    ) -> Self {
        Self {
            pieces,
            new_pieces: Vec::new(),
            assigned_pieces,
            complete_pieces,
            info,
            request_size,
        }
    }
}

impl<'a> PieceIterator<'a> {
    pub fn piece_next(&'a self, index: u32, pieces: &Vec<PieceRequest>) -> Option<PieceRequest> {
        let total_length = self.info.piece_length(index);

        // First check if we need to seed the first piece
        if pieces.len() == 0 {
            let length = {
                if self.request_size < total_length {
                    self.request_size
                } else {
                    total_length
                }
            };

            trace!("Seeding piece {}", index);

            return Some(PieceRequest {
                index,
                begin: 0,
                length,
            });
        }

        // Then check if need to fill gap between piece requests
        for i in 0..pieces.len() - 1 {
            let this_piece = &pieces[i];
            let next_piece = &pieces[i + 1];

            if this_piece.index == next_piece.index
                && this_piece.begin + this_piece.length < next_piece.begin
            {
                trace!("Looking at gap {:?} {:?}", this_piece, next_piece);
                let length = {
                    if this_piece.begin + self.request_size < next_piece.begin {
                        self.request_size
                    } else {
                        next_piece.begin - (this_piece.begin + this_piece.length)
                    }
                };

                let piece_request = PieceRequest {
                    index: this_piece.index,
                    begin: this_piece.begin + this_piece.length,
                    length,
                };

                trace!("Filling gap with {:?}", piece_request);

                return Some(piece_request);
            }
        }

        // Reached end of piece requests
        let final_piece = &pieces[pieces.len() - 1];
        if final_piece.begin + final_piece.length < total_length {
            let length = {
                if final_piece.begin + final_piece.length + self.request_size < total_length {
                    self.request_size
                } else {
                    total_length - (final_piece.begin + final_piece.length)
                }
            };

            let piece_request = PieceRequest {
                index: final_piece.index,
                begin: final_piece.begin + final_piece.length,
                length,
            };

            trace!("Adding end piece {:?}", piece_request);
            trace!(
                "Piece length is {}",
                self.info.piece_length(piece_request.index)
            );

            return Some(piece_request);
        }

        trace!("Yep. Nuffink");

        None
    }
}

impl<'a> Iterator for PieceIterator<'a> {
    type Item = PieceRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let mut piece_indexes: Vec<u32> = self
            .pieces
            .iter()
            .map(|p| p.index)
            .chain(self.new_pieces.iter().map(|p| p.index))
            .collect();
        piece_indexes.sort();
        piece_indexes.dedup();

        trace!("Looking at filling piece indexes: {:?}", piece_indexes);

        let everything: Vec<PieceRequest> = self
            .pieces
            .iter()
            .cloned()
            .chain(self.new_pieces.iter().cloned())
            .collect();

        // We are looking at a combination of pieces
        // Everything = pieces we have + pieces we already requested + pieces we're about to request
        for index in piece_indexes.iter() {
            if self.complete_pieces.contains(&index) {
                trace!("Already have piece {}", index);
                continue;
            }

            if let Some(next_request) = self.piece_next(*index, &everything) {
                self.new_pieces.push(next_request.clone());
                return Some(next_request);
            }
        }

        // If we didn't find a piece to fill, look at the assigned pieces and add the first piece
        // It will get filled, if necessary, on next iteration
        let assignable_pieces = self
            .assigned_pieces
            .iter()
            .filter(|i| !piece_indexes.contains(i) && !self.complete_pieces.contains(i));
        for seed_piece_index in assignable_pieces.into_iter() {
            let length = {
                if self.request_size < self.info.piece_length(*seed_piece_index) {
                    self.request_size
                } else {
                    self.info.piece_length(*seed_piece_index)
                }
            };
            let piece_request = PieceRequest {
                index: *seed_piece_index,
                begin: 0,
                length,
            };
            trace!("Seeding piece from assignment: {:?}", piece_request);
            self.new_pieces.push(piece_request.clone());
            return Some(piece_request);
        }

        trace!("No more piece requests to make");

        None
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashSet, sync::Arc};

    use crate::{
        bencoding::Position,
        metadata::{CompletePiece, Info, ModeFile, Piece, PieceRequest},
        piece_tracker::{self, PIECE_REQUEST_SIZE, PieceIterator, PieceTracker},
    };

    #[test]
    pub fn test_piece_iterator_zero_pieces() {
        let info = info(1024);
        let pieces = vec![];
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            PIECE_REQUEST_SIZE,
        );

        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_one_piece() {
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 500,
        }];
        let info = info(1024);
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            PIECE_REQUEST_SIZE,
        );

        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 500,
                length: 524
            })
        );
        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_one_piece_end() {
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 500,
        }];
        let info = info(500);
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            PIECE_REQUEST_SIZE,
        );

        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_one_piece_two_new_pieces() {
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 25,
        }];
        let info = info(75);
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            25,
        );

        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 25,
                length: 25
            })
        );
        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 50,
                length: 25,
            })
        );
        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_two_pieces_one_new_piece_in_gap() {
        let pieces = vec![
            PieceRequest {
                index: 0,
                begin: 0,
                length: 25,
            },
            PieceRequest {
                index: 0,
                begin: 50,
                length: 25,
            },
        ];
        let info = info(75);
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            25,
        );

        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 25,
                length: 25
            })
        );
        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_two_pieces_two_new_pieces_in_two_gap() {
        let pieces = vec![
            PieceRequest {
                index: 0,
                begin: 0,
                length: 25,
            },
            PieceRequest {
                index: 0,
                begin: 75,
                length: 25,
            },
        ];
        let assigned_piece_indexes = vec![];
        let info = info(100);
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            25,
        );

        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 25,
                length: 25
            })
        );
        assert_eq!(
            piece_iterator.next(),
            Some(PieceRequest {
                index: 0,
                begin: 50,
                length: 25,
            })
        );
        assert_eq!(piece_iterator.next(), None,);
    }

    #[test]
    pub fn test_piece_iterator_fills_one_piece_awkward_numbers() {
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 20,
        }];
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let info = info(250);
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            33,
        )
        .collect();
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 20,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 53,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 86,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 119,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 152,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 185,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 218,
                length: 32,
            },
        ];

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_iterator_fills_two_pieces_awkward_numbers() {
        let pieces = vec![
            PieceRequest {
                index: 0,
                begin: 0,
                length: 20,
            },
            PieceRequest {
                index: 1,
                begin: 0,
                length: 50,
            },
        ];
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let info = info(100);
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            33,
        )
        .collect();
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 20,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 53,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 86,
                length: 14,
            },
            PieceRequest {
                index: 1,
                begin: 50,
                length: 33,
            },
            PieceRequest {
                index: 1,
                begin: 83,
                length: 17,
            },
        ];

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_iterator_fills_one_assigned_piece() {
        let assigned_piece_indexes = vec![0];
        let complete_pieces = HashSet::new();
        let info = info(100);
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &Vec::new(),
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            33,
        )
        .collect();
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 0,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 33,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 66,
                length: 33,
            },
            PieceRequest {
                index: 0,
                begin: 99,
                length: 1,
            },
        ];

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_iterator_fills_one_piece_one_assigned_piece() {
        let assigned_piece_indexes = vec![1];
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 33,
        }];
        let complete_pieces = HashSet::new();
        let info = info(101);
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            55,
        )
        .collect();
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 33,
                length: 55,
            },
            PieceRequest {
                index: 0,
                begin: 88,
                length: 13,
            },
            PieceRequest {
                index: 1,
                begin: 0,
                length: 55,
            },
            PieceRequest {
                index: 1,
                begin: 55,
                length: 46,
            },
        ];

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_iterator_fills_last_piece_correctly() {
        let assigned_piece_indexes = vec![1];
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 33,
        }];
        let complete_pieces = HashSet::new();
        let info = info(101);
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            55,
        )
        .collect();
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 33,
                length: 55,
            },
            PieceRequest {
                index: 0,
                begin: 88,
                length: 13,
            },
            PieceRequest {
                index: 1,
                begin: 0,
                length: 55,
            },
            PieceRequest {
                index: 1,
                begin: 55,
                length: 46,
            },
        ];

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_iterator_fills_entire_piece_correctly_the_bear() {
        let assigned_piece_indexes = vec![0];
        let complete_pieces = HashSet::new();
        let piece_length = 1044480;

        let existing_pieces = vec![
            PieceRequest {
                index: 0,
                begin: 0,
                length: 1015808,
            },
            PieceRequest {
                index: 0,
                begin: 1015808,
                length: 16389,
            },
            PieceRequest {
                index: 0,
                begin: 1015813,
                length: 16384,
            },
        ];
        let expected_pieces: Vec<PieceRequest> = vec![
            PieceRequest {
                index: 0,
                begin: 1032197,
                length: 12283,
            }, // would complete it correctly
        ];
        let info = info(piece_length);

        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &existing_pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            &info,
            PIECE_REQUEST_SIZE,
        )
        .collect();

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_tracker_requests_gap_intersect_left() {
        let assigned_piece_indexes = vec![0];
        let piece_length = PIECE_REQUEST_SIZE * 3;
        let mut piece_tracker = PieceTracker::new(info(piece_length));
        let first_piece_requests = piece_tracker.next_pieces(&assigned_piece_indexes, 100);
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 0,
            block: vec![0u8; PIECE_REQUEST_SIZE as usize + 10],
        });
        let second_piece_requests = piece_tracker.next_pieces(&assigned_piece_indexes, 100);

        assert_eq!(
            first_piece_requests,
            vec![
                PieceRequest {
                    index: 0,
                    begin: 0,
                    length: PIECE_REQUEST_SIZE
                },
                PieceRequest {
                    index: 0,
                    begin: PIECE_REQUEST_SIZE,
                    length: PIECE_REQUEST_SIZE
                },
                PieceRequest {
                    index: 0,
                    begin: PIECE_REQUEST_SIZE * 2,
                    length: PIECE_REQUEST_SIZE
                },
            ]
        );
        assert_eq!(
            second_piece_requests,
            vec![PieceRequest {
                index: 0,
                begin: PIECE_REQUEST_SIZE + 10,
                length: PIECE_REQUEST_SIZE - 10
            },]
        );
    }

    #[test]
    pub fn test_piece_tracker_reports_zero_complete_pieces() {
        let mut piece_tracker = PieceTracker::new(info(4));
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 0,
            block: vec![0, 1],
        });

        assert!(!piece_tracker.is_complete(0));
        assert_eq!(piece_tracker.complete_pieces(), vec![]);
    }

    #[test]
    pub fn test_piece_tracker_reports_one_complete_piece() {
        let mut piece_tracker = PieceTracker::new(info(4));
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 0,
            block: vec![0, 1],
        });
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 2,
            block: vec![2, 3],
        });

        assert!(piece_tracker.is_complete(0));
        assert_eq!(
            piece_tracker.complete_pieces(),
            vec![CompletePiece {
                index: 0,
                data: Arc::from(vec![0, 1, 2, 3]),
            }]
        );
    }

    #[test]
    pub fn test_piece_tracker_reports_zero_complete_pieces_after_removing_complete() {
        let mut piece_tracker = PieceTracker::new(info(4));
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 0,
            block: vec![0, 1],
        });
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 2,
            block: vec![1, 2],
        });
        piece_tracker.mark_complete(0);
        piece_tracker.remove(0);

        assert_eq!(piece_tracker.complete_pieces(), vec![]);
    }

    #[test]
    pub fn test_final_piece_real_piece() {
        // 1032192
        // 1044480
        let mut info = info(4);
        info.pieces_length = 1044480;
        let mut piece_tracker = PieceTracker::new(info);
        piece_tracker.add_piece(Piece {
            index: 0,
            begin: 0,
            block: vec![0u8; 1_032_192],
        });

        assert_eq!(
            piece_tracker.next_pieces(&vec![0], 10),
            vec![PieceRequest {
                index: 0,
                begin: 1_032_192,
                length: 12_288
            }]
        );
    }

    fn info(pieces_length: u32) -> Info {
        Info {
            pieces_length,
            pieces: vec![vec![0].into_iter().cycle().take(20).collect()],
            file_info: ModeFile::Single { length: 10 },
            name: String::from("test.txt").into_bytes(),
            position: Position {
                offset: 0,
                length: 10,
            },
        }
    }
}
