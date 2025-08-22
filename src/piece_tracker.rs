use std::cmp::min;

use crate::metadata::{Piece, PieceRequest};

#[derive(Debug)]
pub struct PieceTracker {
    pieces: Vec<Piece>,
    requested_pieces: Vec<PieceRequest>,
    piece_length: u32,
}

const PIECE_REQUEST_SIZE: u32 = 16_384;

impl PieceTracker {
    pub fn new(piece_length: u32) -> Self {
        Self {
            pieces: vec![],
            requested_pieces: vec![],
            piece_length,
        }
    }

    pub fn next_pieces(&mut self, assigned_piece_indexes: &[u32], max: usize) -> &[PieceRequest] {
        let mut pieces: Vec<PieceRequest> = self
            .pieces
            .iter()
            .map(|stored_piece| PieceRequest {
                index: stored_piece.index,
                begin: stored_piece.begin,
                length: stored_piece.block.len() as u32,
            })
            .chain(self.requested_pieces.iter().cloned())
            .collect();

        pieces.sort_by(|piece0, piece1| {
            (piece0.index, piece1.begin).cmp(&(piece1.index, piece1.begin))
        });

        let mut new_pieces = Vec::new();

        if !pieces.is_empty() {
            for i in 0..pieces.len() - 1 {
                let piece0 = &pieces[i];
                let piece1 = &pieces[i + 1];

                // fill gap between same piece indexes
                if piece0.index == piece1.index && piece0.begin + piece0.length < piece1.begin {
                    let mut gap = fill_gap(piece0, piece1.begin, max);
                    new_pieces.append(&mut gap);
                }

                // fill gap between different piece indexes
                if piece0.index != piece1.index && piece0.begin + piece0.length < self.piece_length
                {
                    let mut gap = fill_gap(piece0, piece1.begin, max);
                    new_pieces.append(&mut gap);
                }
            }
        }

        // new_pieces will be empty if we do not have any pieces whatsoever
        // in that case, seed a piece and fill
        if new_pieces.len() < max {
            for piece_index in assigned_piece_indexes.iter() {
                let piece = PieceRequest {
                    index: *piece_index,
                    begin: 0,
                    length: min(PIECE_REQUEST_SIZE, self.piece_length),
                };
                let mut gap = fill_gap(&piece, self.piece_length, max);
                gap.insert(0, piece);
                new_pieces.append(&mut gap);
            }
        }

        let new_piece_count = new_pieces.len();
        let total_piece_count = new_pieces.len();
        self.requested_pieces.append(&mut new_pieces);

        &self.requested_pieces[(total_piece_count - new_piece_count)..total_piece_count]
    }

    pub fn complete_pieces(&mut self) -> Vec<Piece> {
        unimplemented!()
    }

    pub fn clear_requests(&mut self) {
        self.requested_pieces = vec![];
    }
}

pub fn fill_gap(piece0: &PieceRequest, until_offset: u32, max: usize) -> Vec<PieceRequest> {
    let mut new_pieces = Vec::new();
    let mut offset: u32 = piece0.length;
    let mut count = 0;

    println!(
        "Filling gap index={} offset={} until_offset={} count={} max={}",
        piece0.index, offset, until_offset, count, max
    );

    // Add pieces until gap closes
    while offset < until_offset && count < max {
        // Either request a standard piece size, or when near end, what remains
        let length = min(PIECE_REQUEST_SIZE, until_offset - offset);
        println!("length = {}", length);

        new_pieces.push(PieceRequest {
            index: piece0.index,
            begin: offset,
            length,
        });

        offset += length;

        count += 1;
    }

    new_pieces
}

#[cfg(test)]
mod test {
    use crate::metadata::PieceRequest;
    use crate::piece_bucket::{PIECE_REQUEST_SIZE, PieceTracker, fill_gap};

    #[test]
    fn test_fill_gap_one_piece() {
        let piece = PieceRequest {
            index: 0,
            begin: 0,
            length: PIECE_REQUEST_SIZE / 2,
        };

        assert_eq!(
            fill_gap(&piece, PIECE_REQUEST_SIZE, 2),
            vec![PieceRequest {
                index: 0,
                begin: PIECE_REQUEST_SIZE / 2,
                length: PIECE_REQUEST_SIZE / 2
            }]
        );
    }

    #[test]
    fn test_fill_gap_three_quarters() {
        let piece = PieceRequest {
            index: 0,
            begin: 0,
            length: PIECE_REQUEST_SIZE / 4,
        };

        assert_eq!(
            fill_gap(&piece, PIECE_REQUEST_SIZE, 4),
            vec![PieceRequest {
                index: 0,
                begin: PIECE_REQUEST_SIZE / 4,
                length: 3 * PIECE_REQUEST_SIZE / 4
            },]
        );
    }

    #[test]
    fn test_no_assigned_pieces() {
        let mut piece_bucket = PieceTracker::new(1024);

        assert_eq!(piece_bucket.next_pieces(vec![], 10), &[]);
    }

    #[test]
    fn test_fills_one_piece_from_one_assigned_pieces() {
        let mut piece_bucket = PieceTracker::new(PIECE_REQUEST_SIZE);

        assert_eq!(
            piece_bucket.next_pieces(vec![0], 10),
            &[PieceRequest {
                index: 0,
                begin: 0 * PIECE_REQUEST_SIZE,
                length: PIECE_REQUEST_SIZE,
            },]
        );
    }

    #[test]
    fn test_fills_two_pieces_from_two_assigned_pieces() {
        let mut piece_bucket = PieceTracker::new(PIECE_REQUEST_SIZE);

        assert_eq!(
            piece_bucket.next_pieces(vec![0, 1], 10),
            &[
                PieceRequest {
                    index: 0,
                    begin: 0,
                    length: PIECE_REQUEST_SIZE,
                },
                PieceRequest {
                    index: 1,
                    begin: 0,
                    length: PIECE_REQUEST_SIZE,
                },
            ]
        );
    }

    #[test]
    fn test_fills_four_pieces_from_two_assigned_pieces() {
        let mut piece_bucket = PieceTracker::new(PIECE_REQUEST_SIZE * 2);

        assert_eq!(
            piece_bucket.next_pieces(vec![0, 1], 10),
            &[
                PieceRequest {
                    index: 0,
                    begin: 0,
                    length: PIECE_REQUEST_SIZE,
                },
                PieceRequest {
                    index: 0,
                    begin: PIECE_REQUEST_SIZE,
                    length: PIECE_REQUEST_SIZE,
                },
                PieceRequest {
                    index: 1,
                    begin: 0,
                    length: PIECE_REQUEST_SIZE,
                },
                PieceRequest {
                    index: 1,
                    begin: PIECE_REQUEST_SIZE,
                    length: PIECE_REQUEST_SIZE,
                },
            ]
        );
    }

    /*#[test]
    fn test_fills_four_pieces_from_one_assigned_pieces() {
        let mut piece_bucket = PieceTracker::new(4 * PIECE_REQUEST_SIZE);

        assert_eq!(piece_bucket.next_pieces(vec![0], 4), &[
            PieceRequest {
               index: 0,
               begin: 0 * PIECE_REQUEST_SIZE,
               length: PIECE_REQUEST_SIZE,
            },
            PieceRequest {
               index: 0,
               begin: 1 * PIECE_REQUEST_SIZE,
               length: PIECE_REQUEST_SIZE,
            },
            PieceRequest {
               index: 0,
               begin: 2 * PIECE_REQUEST_SIZE,
               length: PIECE_REQUEST_SIZE,
            },
            PieceRequest {
               index: 0,
               begin: 3 * PIECE_REQUEST_SIZE,
               length: PIECE_REQUEST_SIZE,
            },
        ]);
    }*/
}
