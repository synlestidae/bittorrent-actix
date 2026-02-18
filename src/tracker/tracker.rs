use crate::bittorrent::Bencoding;
use crate::bittorrent::Metadata;
use crate::bittorrent::ParseErr;
use crate::bittorrent::Uri;
use crate::tracker::udp_client::ANNOUNCE_ACTION;
use crate::tracker::udp_client::AnnounceReq;
use crate::tracker::udp_client::AnnounceRes;
use crate::tracker::udp_client::CONNECT_ACTION;
use crate::tracker::udp_client::ConnectReq;
use crate::tracker::udp_client::ConnectRes;
use crate::tracker::udp_client::NONE_EVENT;
use crate::tracker::udp_client::UDP_PROTOCOL_ID;
use crate::tracker::udp_client::UdpClient;
use crate::tracker::udp_client::UdpClientErr;
use actix::prelude::*;
use log::{debug, error, trace};
use percent_encoding::{NON_ALPHANUMERIC, percent_encode};
use rand::RngCore;
use reqwest::StatusCode;
use reqwest::get;
use std::{
    io::{BufReader, Cursor},
    net::{Ipv4Addr, Ipv6Addr},
};

pub struct Tracker {
    metadata: Option<Metadata>,
    udp_address: Option<(String, u16)>, // (host, port)
    info_hash: [u8; 20],
    peer_id: [u8; 20],
}

impl Tracker {
    pub fn with_metadata(metadata: Metadata, info_hash: [u8; 20]) -> Self {
        let mut peer_id = [0u8; 20];
        rand::rng().fill_bytes(&mut peer_id);

        Self {
            metadata: Some(metadata),
            udp_address: None,
            info_hash,
            peer_id,
        }
    }

    pub fn with_uri(uri: String, info_hash: [u8; 20]) -> Self {
        debug!("Tracker URI {}", uri);

        let mut peer_id = [0u8; 20];
        rand::rng().fill_bytes(&mut peer_id);

        let uri = Uri::parse(&uri).unwrap(); // TODO

        println!("Tracker URI parsed {:?}", uri);

        if (uri.scheme != "udp") {
            todo!("handle error when scheme is not udp");
        }

        let host = match uri.host {
            Some(host) => host,
            None => todo!("handle error when host is blank"),
        };

        let port = match uri.port {
            Some(port) => port,
            None => 80,
        };

        Self {
            metadata: None,
            udp_address: Some((host, port)),
            info_hash,
            peer_id,
        }
    }

    fn ask_http(
        &mut self,
        _ctx: &mut Context<Self>,
    ) -> ResponseActFuture<Self, Result<HttpTrackerResponse, TrackerError>> {
        // Build the URL for GET request
        let metadata = match &self.metadata {
            Some(m) => m,
            None => todo!("shouldn't happen"),
        };
        let mut url = metadata.announce.clone();
        let left = metadata.total_file_bytes();

        let info_hash = self.info_hash.clone();
        let peer_id = self.peer_id.clone();

        Box::pin(
            async move {
                url.push_str("?info_hash=");
                url.push_str(&percent_encode(&info_hash, NON_ALPHANUMERIC).to_string());

                url.push_str("&peer_id=");
                url.push_str(&percent_encode(&peer_id, NON_ALPHANUMERIC).to_string());

                url.push_str("&port=6881");
                url.push_str("&uploaded=0");
                url.push_str("&downloaded=0");
                url.push_str(&format!("&left={}", left));
                url.push_str("&compact=1");
                url.push_str("&no_peer_id=0");
                url.push_str("&event=started");

                debug!("Starting tracker request GET {}", url);

                // Perform the request
                let response = get(url).await?;
                if response.status() != StatusCode::OK {
                    error!("Invalid status code from tracker: {}", response.status());
                    return Err(TrackerError::InvalidStatus(response.status()));
                }
                let body = response.bytes().await?;
                let data = body.to_vec();

                let cursor = Cursor::new(data);
                let mut buf_reader = BufReader::new(cursor);
                let bencoding = Bencoding::deserialise(&mut buf_reader).unwrap();
                let tracker_response = parse_tracker_response(bencoding)?;
                debug!("Tracker response: {:?}", tracker_response);

                Ok(tracker_response)
            }
            .into_actor(self),
        )
    }

    fn ask_udp(
        &mut self,
        _ctx: &mut Context<Self>,
    ) -> ResponseActFuture<Self, Result<HttpTrackerResponse, TrackerError>> {
        let info_hash = self.info_hash.clone();
        let peer_id = self.peer_id.clone();

        let (host, port) = match &self.udp_address {
            Some((h, p)) => (h.clone(), *p),
            None => todo!("handle error when udp address somehow not available"),
        };

        Box::pin(
            async move {
                debug!("Connecting to {}", host);

                let mut rng = rand::rng();
                let transaction_id = rng.next_u32();
                let key = rng.next_u32();
                let num_want = -1;
                let client = UdpClient::connect(&host, port).await?;

                debug!("Sending connect request to {}", host);
                let connect_req = ConnectReq {
                    protocol_id: UDP_PROTOCOL_ID,
                    action: CONNECT_ACTION,
                    transaction_id,
                };
                client.send(connect_req).await?;

                debug!("Receiving connect response from {}", host);
                let connect_res: ConnectRes = client.recv().await?;

                debug!("Received connect response from {}", host);
                if connect_res.transaction_id != transaction_id {
                    todo!(
                        "Transaction ID does not match - expected {}, got {}",
                        transaction_id,
                        connect_res.transaction_id
                    );
                }
                let connection_id = connect_res.connection_id;

                debug!("Sending announce req to {}", host);
                let announce_req = AnnounceReq {
                    connection_id,
                    action: ANNOUNCE_ACTION,
                    transaction_id,
                    info_hash: info_hash,
                    peer_id: peer_id,
                    downloaded: 0, // TODO
                    left: 0,       // TODO
                    uploaded: 0,
                    event: NONE_EVENT,
                    ip_address: 0,
                    key,
                    num_want,
                    port,
                };
                client.send(announce_req).await?;

                debug!("Waiting for announce res from {}", host);
                let announce_res: AnnounceRes = client.recv().await?;

                debug!("Received announce res from {}", host);
                trace!("Announce res from {} was {:?}", host, announce_res);

                Ok(HttpTrackerResponse {
                    interval: Some(announce_res.interval),
                    min_interval: None,
                    complete: None,
                    incomplete: None,
                    downloaded: None,
                    peers: announce_res.peers,
                    peers6: vec![],
                    failure_reason: None,
                })
            }
            .into_actor(self),
        )
    }
}

impl Actor for Tracker {
    type Context = Context<Self>;
}

#[derive(Message, Debug)]
#[rtype(result = "Result<HttpTrackerResponse, TrackerError>")]
pub struct AskTracker {}

#[derive(Debug)]
pub enum TrackerError {
    HttpError(reqwest::Error),
    InvalidStatus(StatusCode),
    InvalidUri,
    InvalidUriScheme,
    UdpClientErr(UdpClientErr),
}

impl From<reqwest::Error> for TrackerError {
    fn from(err: reqwest::Error) -> Self {
        TrackerError::HttpError(err)
    }
}

impl From<ParseErr> for TrackerError {
    fn from(_err: ParseErr) -> Self {
        TrackerError::InvalidUri
    }
}

impl From<UdpClientErr> for TrackerError {
    fn from(err: UdpClientErr) -> Self {
        TrackerError::UdpClientErr(err)
    }
}

impl Handler<AskTracker> for Tracker {
    type Result = ResponseActFuture<Self, Result<HttpTrackerResponse, TrackerError>>;

    fn handle(&mut self, _msg: AskTracker, ctx: &mut Context<Self>) -> Self::Result {
        if self.udp_address.is_some() {
            Box::pin(self.ask_udp(ctx))
        } else if self.metadata.is_some() {
            Box::pin(self.ask_http(ctx))
        } else {
            unreachable!("should always have either udp address or metadata")
        }
    }
}

fn parse_tracker_response(body: Bencoding) -> Result<HttpTrackerResponse, TrackerError> {
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

            Ok(HttpTrackerResponse {
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

#[derive(Debug, Clone)]
pub struct HttpTrackerResponse {
    pub interval: Option<u32>,
    pub min_interval: Option<u32>,
    pub complete: Option<u32>,
    pub incomplete: Option<u32>,
    pub downloaded: Option<u32>,
    pub peers: Vec<(Ipv4Addr, u16)>,
    pub peers6: Vec<(Ipv6Addr, u16)>,
    pub failure_reason: Option<String>,
}
