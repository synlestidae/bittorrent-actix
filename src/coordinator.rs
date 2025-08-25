use core::error;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use bitvec::prelude::*;
use log::{debug, error, warn};
use rand::Rng;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::filer::{Filer, PieceRequest};
use crate::metadata::Info;
use crate::peer::{BittorrentMessage, Peer, PeerControl, PeerError};
use crate::peer_state::PeerState;
use crate::tracker::TrackerResponse;

const TIMEOUT_CONNECT_SECS: u64 = 5;

pub struct Coordinator {
    filer_addr: Addr<Filer>,
    peers: HashMap<(Ipv4Addr, u16), (Addr<Peer>, PeerState)>,
    info: Info,
    info_hash: [u8; 20],
    we_have: BitVec<u8, Msb0>,
}

impl Coordinator {
    pub fn new(info: Info, info_hash: [u8; 20], filer_addr: Addr<Filer>) -> Self {
        let pieces_count = info.pieces.len();

        Self {
            filer_addr,
            peers: HashMap::new(),
            info,
            info_hash,
            we_have: BitVec::repeat(false, pieces_count),
        }
    }

    fn control_messages(&mut self) -> Vec<(Ipv4Addr, u16, PeerControl)> {
        let mut messages: Vec<(Ipv4Addr, u16, PeerControl)> = Vec::new();

        // first show we are interested
        for ((ip_address, port), (_, peer_state)) in self
            .peers
            .iter_mut()
            .filter(|(_, (_, peer_state))| peer_state.assigned_pieces().len() == 0)
        {
            messages.push((*ip_address, *port, PeerControl::Interested));

            peer_state.us_interested = true;
        }

        // then assign pieces that don't have one already
        for ((ip_address, port), (_, peer_state)) in self
            .peers
            .iter_mut()
            .filter(|(_, (_, peer_state))| peer_state.assigned_pieces().len() == 0)
        {
            let assigned_pieces: Vec<u32> =
                peer_state.they_have.iter_ones().map(|i| i as u32).collect();
            peer_state.assign_pieces(assigned_pieces.clone());
            messages.push((
                *ip_address,
                *port,
                PeerControl::AssignPieces { assigned_pieces },
            ));
        }

        // then unchoke all
        for ((ip_address, port), (_, peer_state)) in self
            .peers
            .iter_mut()
            .filter(|(_, (_, peer_state))| peer_state.assigned_pieces().len() == 0)
        {
            messages.push((*ip_address, *port, PeerControl::UnchokeThem));

            peer_state.us_choking = false;
        }

        messages
    }
}

impl Actor for Coordinator {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct TrackerUpdate {
    pub tracker_response: TrackerResponse,
}

#[derive(Message, Debug)]
#[rtype(result = "Result<(), PeerError>")]
pub struct NewPeer {
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct PeerMessage {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub msg: BittorrentMessage,
}

impl Handler<TrackerUpdate> for Coordinator {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: TrackerUpdate, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received TrackerUpdate");
        let new_peers: Vec<_> = msg
            .tracker_response
            .peers
            .into_iter()
            .filter(|(ip_address, port)| !self.peers.contains_key(&(*ip_address, *port)))
            .collect();

        debug!("Tracker gave us {} new peers", new_peers.len());

        for (ip_address, port) in new_peers.iter() {
            ctx.address().do_send(NewPeer {
                ip_address: *ip_address,
                port: *port,
            });
        }

        Ok(())
    }
}

impl Handler<NewPeer> for Coordinator {
    type Result = ResponseActFuture<Self, Result<(), PeerError>>;

    fn handle(&mut self, msg: NewPeer, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received {:?}", msg);
        let coordinator_addr = ctx.address();
        let peer_info = self.info.clone();
        let peer_state_info = self.info.clone();
        let info_hash = self.info_hash.clone();

        let ip_address = msg.ip_address;
        let port = msg.port;

        let mut peer_id: [u8; 20] = [0u8; 20];
        let mut rng = rand::rng();
        rng.fill(&mut peer_id);

        Box::pin(
            async move {
                debug!("Connecting to {}:{}", msg.ip_address, msg.port);

                let tcp_stream = match timeout(
                    Duration::from_secs(TIMEOUT_CONNECT_SECS),
                    TcpStream::connect((msg.ip_address, msg.port)),
                )
                .await
                {
                    Ok(Ok(t)) => t,
                    Ok(Err(e)) => return Err(e.into()),
                    Err(_) => return Err(PeerError::Timeout),
                };

                debug!("Connected to {}:{}", msg.ip_address, msg.port);

                Ok(tcp_stream)
            }
            .into_actor(self)
            .map(
                move |tcp_stream_result: Result<TcpStream, PeerError>, actor, _ctx| {
                    debug!("Initialising peer {}:{}", msg.ip_address, msg.port);

                    let tcp_stream = match tcp_stream_result {
                        Ok(t) => t,
                        Err(err) => {
                            error!("Peer {}:{} had error: {:?}", msg.ip_address, msg.port, err);
                            return Err(PeerError::from(err));
                        }
                    };
                    let peer_addr = Peer::create(|ctx| {
                        Peer::new(
                            msg.ip_address,
                            msg.port,
                            coordinator_addr.clone(),
                            &peer_info,
                            info_hash,
                            peer_id,
                            ctx,
                            tcp_stream,
                        )
                    });

                    debug!("Initialised peer {}:{}", msg.ip_address, msg.port);

                    let peer_state = PeerState::new(&peer_state_info);
                    actor
                        .peers
                        .insert((ip_address, port), (peer_addr, peer_state));

                    let control_messages = actor.control_messages();
                    for (ip_address, port, message) in control_messages.into_iter() {
                        if let Some((peer_addr, _)) = actor.peers.get(&(ip_address, port)) {
                            peer_addr.do_send(message);
                        }
                    }

                    Ok(())
                },
            ),
        )
    }
}

impl Handler<PeerMessage> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: PeerMessage, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received PeerMessage from {}:{}", msg.ip_address, msg.port);
        debug!("PeerMessage was: {:?}", msg.msg);

        let (ip_address, port) = (msg.ip_address, msg.port);

        let (peer_addr, peer_state) = match self.peers.get_mut(&(ip_address, port)) {
            Some((peer_addr, peer_state)) => (peer_addr.clone(), peer_state),
            None => {
                warn!("Peer {}:{} not found", ip_address, port);
                return;
            }
        };

        peer_state.new_message(&msg.msg);

        if peer_state.assigned_pieces().len() == 0 {
            let assigned_pieces: Vec<u32> =
                peer_state.they_have.iter_ones().map(|i| i as u32).collect();
            debug!(
                "Assigning {} pieces to {}:{}",
                assigned_pieces.len(),
                ip_address,
                port,
            );
            peer_state.assign_pieces(assigned_pieces.clone());
            peer_addr.do_send(PeerControl::AssignPieces { assigned_pieces });
        }

        if let Some((peer_addr, state)) = self.peers.get(&(ip_address, port))
            && let BittorrentMessage::Request {
                index,
                begin,
                length,
            } = msg.msg
            && !state.us_choking
        {
            self.filer_addr.do_send(PieceRequest {
                index,
                begin,
                length,
                ip_address,
                port,
                peer_addr: peer_addr.clone(),
            });
        }

        let control_messages = self.control_messages();
        for (ip_address, port, message) in control_messages.into_iter() {
            if let Some((peer_addr, _)) = self.peers.get(&(ip_address, port)) {
                peer_addr.do_send(message);
            }
        }
    }
}

struct RequestedPiece {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
}
