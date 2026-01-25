use crate::bittorrent::{Magnet, Metadata};

#[derive(Clone)]
pub enum MetadataSource {
    Magnet {
        info_hash: [u8; 20],
        magnet: Magnet,
    },
    Metadata {
        info_hash: [u8; 20],
        metadata: Metadata,
    },
}
