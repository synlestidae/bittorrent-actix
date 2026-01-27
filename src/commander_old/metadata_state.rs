use crate::bittorrent::{ExtensionRequest, Info, Magnet};

pub enum MetadataState {
    HaveMagnet { magnet: MagnetMetadataTracker },
    HaveMetdata { info: Info },
}

pub struct MagnetMetadataTracker {
    magnet: Magnet,
    data: Vec<u8>,
}

const REQUEST_SIZE: u32 = 16 * 1024 * 1024;

impl MagnetMetadataTracker {
    pub fn new(magnet: Magnet) -> Self {
        Self {
            magnet,
            data: vec![],
        }
    }

    pub fn next_request(&self) -> Option<ExtensionRequest> {
        if self.data.len() >= self.magnet.exact_length {
            return None;
        }

        let piece = self.data.len() as u32 / REQUEST_SIZE;

        Some(ExtensionRequest { piece })
    }

    pub fn add_piece(&mut self, index: u32, mut piece: Vec<u8>) -> bool {
        // TODO deal with the index
        if piece.len() + self.data.len() > self.magnet.exact_length {
            return false;
        }
        if piece.len() + self.data.len() == self.magnet.exact_length {
            self.data.append(&mut piece);
            return true;
        }
        if piece.len() as u32 == REQUEST_SIZE {
            self.data.append(&mut piece);
            return true;
        }

        false
    }
}

#[cfg(test)]
mod test {
    use crate::{
        bittorrent::{ExtensionRequest, Magnet},
        commander::metadata_state::MagnetMetadataTracker,
    };

    const REQUEST_SIZE: usize = 16 * 1024 * 1024;

    #[test]
    fn next_request_exact_length_zero() {
        let metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some("No data at all for some reason".to_owned()),
                trackers: vec![],
                exact_length: 0,
            },
            data: vec![],
        };

        assert_eq!(metadata_tracker.next_request(), None);
    }

    #[test]
    fn next_request_factor_request_size() {
        let metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some(
                    "3 pieces already with a fourth piece exactly REQUEST_SIZE".to_owned(),
                ),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 4,
            },
            data: vec![0u8; REQUEST_SIZE * 3],
        };

        assert_eq!(
            metadata_tracker.next_request(),
            Some(ExtensionRequest { piece: 3 })
        );
    }

    #[test]
    fn next_request_remainder_last_piece() {
        let metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some(
                    "3 pieces already with a fourth piece exactly REQUEST_SIZE".to_owned(),
                ),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 4 + 1993,
            },
            data: vec![0u8; REQUEST_SIZE * 4],
        };

        assert_eq!(
            metadata_tracker.next_request(),
            Some(ExtensionRequest { piece: 4 })
        );
    }

    #[test]
    fn next_request_smaller_than_request_size() {
        let metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some(
                    "It's only 5555 bytes which is way less than REQUEST_SIZE".to_owned(),
                ),
                trackers: vec![],
                exact_length: 5555,
            },
            data: vec![0u8; 0],
        };

        assert_eq!(
            metadata_tracker.next_request(),
            Some(ExtensionRequest { piece: 0 })
        );
    }

    #[test]
    fn add_piece_request_size() {
        let mut metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some("No data yet and need four pieces".to_owned()),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 4,
            },
            data: vec![0u8; 0],
        };

        assert!(metadata_tracker.add_piece(vec![0u8; REQUEST_SIZE]));
        assert_eq!(metadata_tracker.data, vec![0u8; REQUEST_SIZE]);
        assert_eq!(
            metadata_tracker.next_request(),
            Some(ExtensionRequest { piece: 1 })
        );
    }

    #[test]
    fn add_last_piece_request_size() {
        let mut metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some("Just need one more piece".to_owned()),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 4,
            },
            data: vec![0u8; REQUEST_SIZE * 3],
        };

        assert!(metadata_tracker.add_piece(vec![0u8; REQUEST_SIZE]));
        assert_eq!(metadata_tracker.data, vec![0u8; REQUEST_SIZE * 4]);
        assert_eq!(metadata_tracker.next_request(), None);
    }

    #[test]
    fn add_last_piece_request_size_remainder_success() {
        let mut metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some(
                    "Just last piece and its not a multiple of REQUEST_SIZE".to_owned(),
                ),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 3 + 1234,
            },
            data: vec![0u8; REQUEST_SIZE * 3],
        };

        assert!(metadata_tracker.add_piece(vec![0u8; 1234]));
        assert_eq!(metadata_tracker.data, vec![0u8; REQUEST_SIZE * 3 + 1234]);
        assert_eq!(metadata_tracker.next_request(), None);
    }

    #[test]
    fn add_last_piece_request_size_remainder_fail() {
        let mut metadata_tracker = MagnetMetadataTracker {
            magnet: Magnet {
                info_hash: [0u8; 20],
                display_name: Some(
                    "Just last piece and its not a multiple of REQUEST_SIZE".to_owned(),
                ),
                trackers: vec![],
                exact_length: REQUEST_SIZE * 3 + 1234,
            },
            data: vec![0u8; REQUEST_SIZE * 3],
        };

        assert!(!metadata_tracker.add_piece(vec![0u8; 4321]));
        assert_eq!(metadata_tracker.data, vec![0u8; REQUEST_SIZE * 3]);
        assert_eq!(
            metadata_tracker.next_request(),
            Some(ExtensionRequest { piece: 3 })
        );
    }
}
