use crate::bittorrent::Metadata;
use crate::tracker::udp_server::UdpClient;


pub struct UdpTracker {
    metadata: Metadata,
    info_hash: [u8; 20],
    peer_id: Option<[u8; 20]>,
}

pub struct UdpTrackerRequest {

}

pub struct UdpTrackerResponse {
}
