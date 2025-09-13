use std::{cmp::min, collections::HashSet};

use crate::metadata::{CompletePiece, Piece, PieceRequest};

#[derive(Debug)]
pub struct PieceTracker {
    pieces: Vec<Piece>,
    requested_pieces: Vec<PieceRequest>,
    piece_length: u32,
    complete_pieces: HashSet<u32>,
}

pub const PIECE_REQUEST_SIZE: u32 = 16_384;

impl PieceTracker {
    pub fn new(piece_length: u32) -> Self {
        Self {
            pieces: vec![],
            requested_pieces: vec![],
            piece_length,
            complete_pieces: HashSet::new(),
        }
    }

    pub fn add_piece(&mut self, piece: Piece) {
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

        self.pieces.push(piece);
    }

    pub fn remove(&mut self, piece_index: u32) {
        self.pieces.retain(|p| p.index != piece_index);
    }

    pub fn is_complete(&self, piece_index: u32) -> bool {
        let mut pieces: Vec<_> = self
            .pieces
            .iter()
            .filter(|p| p.index == piece_index)
            .collect();
        pieces.sort_by_key(|p| p.begin);
        if pieces.len() == 0 {
            return false;
        }

        let mut piece = pieces[0];
        for i in 1..pieces.len() {
            let next_piece = pieces[i];
            dbg!(piece, next_piece);
            if !(piece.begin + piece.block.len() as u32 <= next_piece.begin) {
                return false;
            }
            piece = next_piece;
        }

        let is_complete = piece.begin + piece.block.len() as u32 >= self.piece_length;
        dbg!(piece, self.piece_length, is_complete);
        is_complete
    }

    pub fn mark_complete(&mut self, piece_index: u32) {
        self.complete_pieces.insert(piece_index);
    }

    pub fn get_piece(&self, piece_index: u32) -> Vec<u8> {
        let mut piece = vec![0u8; self.piece_length as usize];
        for sub_piece in self.pieces.iter().filter(|p| p.index == piece_index) {
            let mut i = 0;
            let sub_piece_len = sub_piece.block.len() as u32;
            while i < sub_piece_len && sub_piece.begin + i < piece.len() as u32 {
                piece[(sub_piece.begin + i) as usize] = sub_piece.block[i as usize];
                i += 1;
            }
        }

        piece
    }

    pub fn next_pieces(&mut self, assigned_piece_indexes: &[u32], max: usize) -> Vec<PieceRequest> {
        let max_next = max as isize - self.requested_pieces.len() as isize;
        if max_next <= 0 {
            return vec![];
        }

        let pieces = self
            .pieces
            .iter()
            .map(|p| PieceRequest {
                index: p.index,
                begin: p.begin,
                length: p.block.len() as u32,
            })
            .chain(self.requested_pieces.iter().cloned())
            .collect();

        let piece_iterator = PieceIterator::new(
            &pieces,
            assigned_piece_indexes,
            &self.complete_pieces,
            self.piece_length,
            PIECE_REQUEST_SIZE,
        );

        let new_pieces: Vec<_> = piece_iterator.take(max_next as usize).collect();
        self.requested_pieces.append(&mut new_pieces.clone());

        new_pieces
    }

    pub fn complete_pieces(&mut self) -> Vec<CompletePiece> {
        let piece_indexes = self
            .pieces
            .iter()
            .map(|p| p.index)
            .collect::<HashSet<u32>>();
        let mut complete_pieces = Vec::new();

        for piece_index in piece_indexes.iter() {
            if self.is_complete(*piece_index) {
                let complete_piece = self.get_piece(*piece_index);
                complete_pieces.push(CompletePiece {
                    index: *piece_index,
                    data: complete_piece,
                });
            }
        }

        return complete_pieces;
    }
}

struct PieceIterator<'a> {
    pieces: &'a Vec<PieceRequest>,
    new_pieces: Vec<PieceRequest>,
    assigned_pieces: &'a [u32],
    complete_pieces: &'a HashSet<u32>,
    total_piece_length: u32,
    request_size: u32,
    /// Piece that we analysing now
    piece_vec_index: usize,
    /// Of the piece we analysing now, the offset within that piece
    offset_index: u32,
}

impl<'a> PieceIterator<'a> {
    fn new(
        pieces: &'a Vec<PieceRequest>,
        assigned_pieces: &'a [u32],
        complete_pieces: &'a HashSet<u32>,
        total_piece_length: u32,
        request_size: u32,
    ) -> Self {
        Self {
            pieces,
            new_pieces: Vec::new(),
            assigned_pieces,
            complete_pieces,
            total_piece_length,
            request_size,
            offset_index: 0,
            piece_vec_index: 0,
        }
    }
}

impl<'a> Iterator for PieceIterator<'a> {
    type Item = PieceRequest;

    fn next(&mut self) -> Option<Self::Item> {
        // To analyse gaps, pieces need to be in order
        // Note that biggest pieces go first
        let mut pieces: Vec<_> = self
            .pieces
            .iter()
            .cloned()
            .chain(self.new_pieces.iter().cloned())
            .filter(|p| !self.complete_pieces.contains(&p.index))
            .collect();
        pieces.sort_by_key(|p| (p.index, p.begin, u32::MAX - p.length));

        while self.piece_vec_index + 1 < pieces.len() {
            // We are not at end of pieces
            // We may need to fill a gap
            let piece = &pieces[self.piece_vec_index];
            let next_piece = &pieces[self.piece_vec_index + 1];

            if piece.index == next_piece.index && piece.begin + piece.length < next_piece.begin {
                let begin = piece.begin + piece.length;
                let length = {
                    if begin + self.request_size > next_piece.begin {
                        next_piece.begin - begin
                    } else {
                        self.request_size
                    }
                };
                let piece_request = PieceRequest {
                    index: piece.index,
                    begin,
                    length,
                };

                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            } else if piece.index != next_piece.index
                && piece.begin + piece.length < self.total_piece_length
            {
                let begin = piece.begin + piece.length;
                let length = {
                    if begin + self.request_size > self.total_piece_length {
                        self.total_piece_length - begin
                    } else {
                        self.request_size
                    }
                };
                let piece_request = PieceRequest {
                    index: piece.index,
                    begin,
                    length,
                };

                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            }

            self.piece_vec_index += 1;
        }

        if self.piece_vec_index + 1 == pieces.len() {
            let piece = &pieces[self.piece_vec_index];
            let piece_end = piece.begin + piece.length;
            let new_begin = piece_end;

            dbg!(piece_end, self.total_piece_length);
            if piece_end < self.total_piece_length {
                // Last piece needs filling
                dbg!(new_begin, self.total_piece_length, self.request_size);
                let new_length = {
                    if new_begin + self.request_size > self.total_piece_length {
                        self.total_piece_length - new_begin
                    } else {
                        self.request_size
                    }
                };

                let piece_request = PieceRequest {
                    index: piece.index,
                    begin: new_begin,
                    length: new_length,
                };
                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            }
        }

        // All existing pieces have their gaps filled
        // We can start on the assigned pieces now
        // First find one that we haven't seen yet
        // Create the first and yield it
        for index in self.assigned_pieces.iter() {
            if !pieces.iter().any(|p| p.index == *index) {
                let begin = self.offset_index;
                let length = min(self.total_piece_length, self.request_size);

                self.offset_index += length;

                let piece_request = PieceRequest {
                    index: *index,
                    begin,
                    length,
                };
                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            }
        }

        // No more pieces to do
        None
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use crate::{
        metadata::{CompletePiece, Piece, PieceRequest},
        piece_tracker::{self, PIECE_REQUEST_SIZE, PieceIterator, PieceTracker},
    };

    #[test]
    pub fn test_piece_iterator_zero_pieces() {
        let pieces = vec![];
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            1024,
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
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            1024,
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
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator = PieceIterator::new(
            &pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            500,
            PIECE_REQUEST_SIZE,
        );

        assert_eq!(piece_iterator.next(), None);
    }

    //#[test]
    pub fn test_piece_iterator_one_piece_two_new_pieces() {
        let pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 25,
        }];
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 75, 25);

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
        let mut pieces = vec![
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
        let assigned_piece_indexes = vec![];
        let complete_pieces = HashSet::new();
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 75, 25);

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
        let mut pieces = vec![
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
        let complete_pieces = HashSet::new();
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 100, 25);

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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 250, 33)
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
        let mut pieces = vec![
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 100, 33)
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
        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &Vec::new(),
            &assigned_piece_indexes,
            &complete_pieces,
            100,
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 101, 55)
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, &complete_pieces, 101, 55)
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

        let new_pieces: Vec<PieceRequest> = PieceIterator::new(
            &existing_pieces,
            &assigned_piece_indexes,
            &complete_pieces,
            piece_length,
            PIECE_REQUEST_SIZE,
        )
        .collect();

        assert_eq!(expected_pieces, new_pieces);
    }

    #[test]
    pub fn test_piece_tracker_requests_gap_intersect_left() {
        let assigned_piece_indexes = vec![0];
        let piece_length = PIECE_REQUEST_SIZE * 3;

        let mut piece_tracker = PieceTracker::new(piece_length);
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
        let mut piece_tracker = PieceTracker::new(4);
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
        let mut piece_tracker = PieceTracker::new(4);
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
        assert_eq!(piece_tracker.complete_pieces(), vec![
           CompletePiece {
               index: 0,
               data: vec![0, 1, 2, 3],
           }
        ]);
    }

    #[test]
    pub fn test_piece_tracker_reports_zero_complete_pieces_after_removing_complete() {
        let mut piece_tracker = PieceTracker::new(4);
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
}
