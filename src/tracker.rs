use crate::coordinator::Coordinator;
use crate::peer::{Handshake, Peer};
use crate::{bencoding::Bencoding, tracker};
use actix::prelude::*;
use log::{debug, error};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use rand::RngCore;
use std::{
    io::{BufReader, Cursor},
    net::{Ipv4Addr, Ipv6Addr},
};
use ureq::get;
use ureq::http::StatusCode;

#[derive(Default)]
pub struct Tracker {
    announce_url: Option<String>,
    info_hash: Option<[u8; 20]>,
    coordinator: Option<Addr<Coordinator>>,
    peer_id: Option<[u8; 20]>,
}

impl Actor for Tracker {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<(), TrackerError>")]
pub struct AskTracker {
    pub announce_url: String,
    pub info_hash: [u8; 20],
    pub coordinator: Addr<Coordinator>,
    pub left: u64,
}

pub enum TrackerError {
    HttpError(ureq::Error),
    InvalidStatus(StatusCode),
}

#[derive(Debug)]
pub struct TrackerResponse {
    pub interval: Option<u32>,
    pub min_interval: Option<u32>,
    pub complete: Option<u32>,
    pub incomplete: Option<u32>,
    pub downloaded: Option<u32>,
    pub peers: Vec<(Ipv4Addr, u16)>,
    pub peers6: Vec<(Ipv6Addr, u16)>,
    pub failure_reason: Option<String>,
}

impl From<ureq::Error> for TrackerError {
    fn from(err: ureq::Error) -> Self {
        TrackerError::HttpError(err)
    }
}

impl Handler<AskTracker> for Tracker {
    type Result = Result<(), TrackerError>;

    fn handle(&mut self, msg: AskTracker, ctx: &mut Context<Self>) -> Self::Result {
        self.announce_url = Some(msg.announce_url.clone());
        self.info_hash = Some(msg.info_hash.clone());
        self.coordinator = Some(msg.coordinator.clone());

        // Build the URL for GET request
        let mut url = msg.announce_url.clone();

        url.push_str("?info_hash=");
        url.push_str(&percent_encode(&msg.info_hash, NON_ALPHANUMERIC).to_string());

        url.push_str("&peer_id=");
        let mut peer_id = [0u8; 20];
        rand::rng().fill_bytes(&mut peer_id);
        url.push_str(&percent_encode(&peer_id, NON_ALPHANUMERIC).to_string());
        self.peer_id = Some(peer_id);

        url.push_str("&port=6881");
        url.push_str("&uploaded=0");
        url.push_str("&downloaded=0");
        url.push_str(&format!("&left={}", msg.left));
        url.push_str("&compact=1");
        url.push_str("&no_peer_id=0");
        url.push_str("&event=started");

        debug!("Starting tracker request GET {}", url);

        // Perform the request
        let response_result = get(url).call();
        let mut response = match response_result {
            Ok(r) => r,
            Err(error) => {
                debug!("HTTP error: {:?}", error);
                return Err(error.into());
            }
        };
        let body = response.body_mut();
        let data = body.read_to_vec()?;
        if response.status() != StatusCode::OK {
            error!("Invalid status code from tracker: {}", response.status());
            return Err(TrackerError::InvalidStatus(response.status()));
        }

        let cursor = Cursor::new(data);
        let mut buf_reader = BufReader::new(cursor);
        let bencoding = Bencoding::deserialize(&mut buf_reader).unwrap();
        let tracker_response = parse_tracker_response(bencoding)?;
        debug!("Tracker response: {:?}", tracker_response);

        for (ip_address, port) in tracker_response.peers.into_iter() {
            let peer_addr = Peer::new(ip_address, port, ctx.address()).start();

            peer_addr.do_send(Handshake {
                info_hash: msg.info_hash,
                peer_id: peer_id,
            });
        }

        //let tracker_response = parse_tracker_response(bencoding);

        // add new peers to dictionary
        // initialise actors for them
        // init the peers if we don't have enough
        Ok(())
    }
}

fn parse_tracker_response(body: Bencoding) -> Result<TrackerResponse, TrackerError> {
    match body {
        Bencoding::Dictionary { dictionary, .. } => {
            let interval = match dictionary.get("interval".as_bytes()) {
                Some(Bencoding::Integer { integer, .. }) => Some(*integer as u32),
                Some(_) => unimplemented!(),
                None => None,
            };

            let min_interval = match dictionary.get("min_interval".as_bytes()) {
                Some(Bencoding::Integer { integer, .. }) => Some(*integer as u32),
                Some(_) => unimplemented!(),
                None => None,
            };

            let complete = match dictionary.get("complete".as_bytes()) {
                Some(Bencoding::Integer { integer, .. }) => Some(*integer as u32),
                Some(_) => unimplemented!(),
                None => None,
            };

            let incomplete = match dictionary.get("incomplete".as_bytes()) {
                Some(Bencoding::Integer { integer, .. }) => Some(*integer as u32),
                Some(_) => unimplemented!(),
                None => None,
            };

            let downloaded = match dictionary.get("downloaded".as_bytes()) {
                Some(Bencoding::Integer { integer, .. }) => Some(*integer as u32),
                Some(_) => unimplemented!(),
                None => None,
            };

            let peers: Vec<(Ipv4Addr, u16)> = match dictionary.get("peers".as_bytes()) {
                Some(Bencoding::ByteString { byte_string, .. }) => byte_string
                    .chunks(6)
                    .map(|address_port| match address_port {
                        [i0, i1, i2, i3, p0, p1] => (
                            Ipv4Addr::new(*i0, *i1, *i2, *i3),
                            ((*p0 as u16) << 8) + (*p1 as u16),
                        ),
                        _ => unimplemented!(),
                    })
                    .collect(),
                Some(_) => unimplemented!(),
                None => vec![],
            };

            let failure_reason = match dictionary.get("failure_reason".as_bytes()) {
                Some(Bencoding::ByteString { byte_string, .. }) => {
                    Some(String::from_utf8_lossy(byte_string).to_string())
                }
                Some(_) => unimplemented!(),
                None => None,
            };

            Ok(TrackerResponse {
                interval,
                min_interval,
                complete,
                incomplete,
                downloaded,
                peers,
                peers6: vec![],
                failure_reason,
            })
        }
        _ => unimplemented!(),
    }
}
