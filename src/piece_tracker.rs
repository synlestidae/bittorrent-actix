use log::trace;
use std::{cmp::min, collections::HashSet};

use crate::metadata::{CompletePiece, Piece, PieceRequest};

#[derive(Debug)]
pub struct PieceTracker {
    pieces: Vec<Piece>,
    requested_pieces: Vec<PieceRequest>,
    piece_length: u32,
    complete_pieces: HashSet<u32>,
}

const PIECE_REQUEST_SIZE: u32 = 16_384;

impl PieceTracker {
    pub fn new(piece_length: u32) -> Self {
        Self {
            pieces: vec![],
            requested_pieces: vec![],
            piece_length,
            complete_pieces: HashSet::new(),
        }
    }

    pub fn add(&mut self, piece: Piece) {
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
        for next_piece in pieces.into_iter() {
            if !(piece.begin + piece.block.len() as u32 >= next_piece.begin) {
                return false;
            }
            piece = next_piece;
        }

        true
    }

    pub fn mark_complete(&mut self, piece_index: u32) {
        self.complete_pieces.insert(piece_index);
    }

    pub fn get_piece(&self, piece_index: u32) -> Vec<u8> {
        let mut piece = vec![0u8; self.piece_length as usize];
        for sub_piece in self.pieces.iter().filter(|p| p.index == piece_index) {
            let start = sub_piece.begin as usize;
            let end = start + sub_piece.block.len();
            for i in start..end {
                piece[i] = sub_piece.block[i];
            }
        }

        piece
    }

    pub fn next_pieces(&mut self, assigned_piece_indexes: &[u32], max: usize) -> Vec<PieceRequest> {
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

        let total_max = max - self.pieces.len();

        let mut new_pieces: Vec<_> = piece_iterator.take(total_max).collect();
        self.requested_pieces.append(&mut new_pieces.clone());

        trace!("Got {} new piece(s) to request", new_pieces.len());

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
                    block: complete_piece,
                });
            }
        }

        return complete_pieces;
    }

    pub fn clear_requests(&mut self) {
        self.requested_pieces = vec![];
    }
}

struct PieceIterator<'a> {
    pieces: &'a Vec<PieceRequest>,
    new_pieces: Vec<PieceRequest>,
    assigned_pieces: &'a [u32],
    complete_pieces: &'a HashSet<u32>,
    piece_length: u32,
    max_piece_length: u32,
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
        piece_length: u32,
        max_piece_length: u32,
    ) -> Self {
        Self {
            pieces,
            new_pieces: Vec::new(),
            assigned_pieces,
            complete_pieces,
            piece_length,
            max_piece_length,
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
            .filter(|p| self.complete_pieces.contains(&p.index))
            .collect();
        pieces.sort_by_key(|p| (p.index, p.begin, u32::MAX - p.length));

        trace!("Pieces to iterate: {:?}", pieces);

        while self.piece_vec_index + 1 < pieces.len() {
            // We are not at end of pieces
            // We may need to fill a gap
            let piece = &pieces[self.piece_vec_index];
            let next_piece = &pieces[self.piece_vec_index + 1];

            trace!("Looking at two pieces: ({:?},{:?})", piece, next_piece);

            let begin = piece.begin + piece.length;
            let length = min(self.max_piece_length, self.piece_length - begin);

            if piece.index == next_piece.index && piece.begin + piece.length < next_piece.begin {
                // We're between two different pieces but first piece does not complete piece
                // request
                trace!(
                    "Filling gap between two different: min({}, {} - {})",
                    self.max_piece_length, self.piece_length, begin
                );

                let piece_request = PieceRequest {
                    index: piece.index,
                    begin,
                    length,
                };

                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            } else if piece.index != next_piece.index
                && piece.begin + piece.length < self.piece_length
            {
                // We're between two same pieces and first piece does not complete piece
                trace!(
                    "Filling gap between two different: min({}, {} - {})",
                    self.max_piece_length, self.piece_length, begin
                );

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
            // We are at the last piece. We may need to complete it

            let piece = &pieces[self.piece_vec_index];
            let piece_end = piece.begin + piece.length;

            trace!("At last piece: {:?} with piece end {}", piece, piece_end);

            if piece_end < self.piece_length {
                // Last piece needs filling
                let length = min(self.max_piece_length, self.piece_length - piece_end);

                let piece_request = PieceRequest {
                    index: piece.index,
                    begin: piece_end,
                    length,
                };
                trace!("Filling last piece: {:?}", piece_request);
                self.new_pieces.push(piece_request.clone());
                return Some(piece_request);
            }

            // Last piece now complete!
            // Can now move to assigned pieces
        }

        // All existing pieces have their gaps filled
        // We can start on the assigned pieces now
        // First find one that we haven't seen yet
        // Create the first and yield it
        trace!("Observing assigned pieces now");
        for index in self.assigned_pieces.iter() {
            if !pieces.iter().any(|p| p.index == *index) {
                let begin = self.offset_index;
                let length = min(self.max_piece_length, self.piece_length);

                self.offset_index += length;

                let piece_request = PieceRequest {
                    index: *index,
                    begin,
                    length,
                };
                self.new_pieces.push(piece_request.clone());
                trace!("Filling assigned piece: {:?}", piece_request);
                return Some(piece_request);
            }
        }

        // No more pieces to do
        None
    }
}

#[cfg(test)]
mod test {
    use crate::{
        metadata::PieceRequest,
        piece_tracker::{PIECE_REQUEST_SIZE, PieceIterator},
    };

    #[test]
    pub fn test_piece_iterator_zero_pieces() {
        let mut pieces = vec![];
        let assigned_piece_indexes = vec![];
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 1024, PIECE_REQUEST_SIZE);

        assert_eq!(piece_iterator.next(), None);
    }

    #[test]
    pub fn test_piece_iterator_one_piece() {
        let mut pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 500,
        }];
        let assigned_piece_indexes = vec![];
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 1024, PIECE_REQUEST_SIZE);

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
        let mut pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 500,
        }];
        let assigned_piece_indexes = vec![];
        let mut piece_iterator =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 500, PIECE_REQUEST_SIZE);

        assert_eq!(piece_iterator.next(), None);
    }

    //#[test]
    pub fn test_piece_iterator_one_piece_two_new_pieces() {
        let mut pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 25,
        }];
        let assigned_piece_indexes = vec![];
        let mut piece_iterator = PieceIterator::new(&pieces, &assigned_piece_indexes, 75, 25);

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
        let mut piece_iterator = PieceIterator::new(&pieces, &assigned_piece_indexes, 75, 25);

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
        let mut piece_iterator = PieceIterator::new(&pieces, &assigned_piece_indexes, 100, 25);

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
        let mut pieces = vec![PieceRequest {
            index: 0,
            begin: 0,
            length: 20,
        }];
        let assigned_piece_indexes = vec![];
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 250, 33).collect();
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 100, 33).collect();
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&Vec::new(), &assigned_piece_indexes, 100, 33).collect();
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
        let new_pieces: Vec<PieceRequest> =
            PieceIterator::new(&pieces, &assigned_piece_indexes, 101, 55).collect();
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
}
