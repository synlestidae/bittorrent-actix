use std::cmp::max;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use log::{debug, error};
use rand::Rng;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::commander::{CommandSet, Commander, CommanderMsg, CommanderMsgEnvelope};
use crate::filer::{Filer, FilerEvent, RequestPieceEnvelope};
use crate::metadata::Info;
use crate::peer::{Peer, PeerControl, PeerControlEnvelope, PeerError};
use crate::peer_state::PeerChange;
use crate::tracker::TrackerResponse;

const TIMEOUT_CONNECT_SECS: u64 = 5;

pub struct Coordinator {
    filer_addr: Addr<Filer>,
    commander_addr: Addr<Commander>,
    peer_addrs: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    info_hash: [u8; 20],
}

impl Coordinator {
    pub fn new(
        info: Info,
        info_hash: [u8; 20],
        filer_addr: Addr<Filer>,
        ctx: &mut Context<Self>,
    ) -> Self {
        let commander_addr = Commander::new(info.clone(), ctx.address()).start();

        Self {
            filer_addr,
            commander_addr,
            peer_addrs: HashMap::new(),
            info_hash,
        }
    }
}

impl Actor for Coordinator {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {}
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

#[derive(Message, Debug)]
#[rtype(result = "()")]
pub struct PeerDisconnect {
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

impl Handler<TrackerUpdate> for Coordinator {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: TrackerUpdate, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received TrackerUpdate");
        let new_peers: Vec<_> = msg
            .tracker_response
            .peers
            .into_iter()
            .filter(|(ip_address, port)| !self.peer_addrs.contains_key(&(*ip_address, *port)))
            .collect();

        ctx.set_mailbox_capacity(max(new_peers.len() * 2, 5));

        debug!("Tracker gave us {} new peers", new_peers.len());

        for (ip_address, port) in new_peers.iter() {
            ctx.spawn(
                ctx.address()
                    .send(NewPeer {
                        ip_address: *ip_address,
                        port: *port,
                    })
                    .into_actor(self)
                    .map(|result, _, _| {
                        if let Err(err) = result {
                            error!("Error creating new peer: {}", err);
                        }
                    }),
            );
        }

        Ok(())
    }
}

impl Handler<NewPeer> for Coordinator {
    type Result = ResponseActFuture<Self, Result<(), PeerError>>;

    fn handle(&mut self, msg: NewPeer, ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received {:?}", msg);
        let coordinator_addr = ctx.address();
        let info_hash = self.info_hash;

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
                move |tcp_stream_result: Result<TcpStream, PeerError>, actor, ctx| {
                    debug!("Initialising peer {}:{}", msg.ip_address, msg.port);

                    let tcp_stream = match tcp_stream_result {
                        Ok(t) => t,
                        Err(err) => {
                            error!("Peer {}:{} had error: {:?}", msg.ip_address, msg.port, err);
                            return Err(err);
                        }
                    };
                    let peer_addr = Peer::create(|ctx| {
                        Peer::new(
                            msg.ip_address,
                            msg.port,
                            coordinator_addr.clone(),
                            info_hash,
                            peer_id,
                            tcp_stream,
                            ctx,
                        )
                    });

                    debug!("Initialised peer {}:{}", msg.ip_address, msg.port);
                    actor.peer_addrs.insert((ip_address, port), peer_addr);
                    ctx.spawn(
                        actor
                            .commander_addr
                            .send(CommanderMsgEnvelope {
                                ip_address,
                                port,
                                msgs: vec![CommanderMsg::AddPeer],
                            })
                            .into_actor(actor)
                            .map(|result, _, _| {
                                if let Err(err) = result {
                                    error!("Error commanding new peer: {}", err);
                                }
                            }),
                    );

                    Ok(())
                },
            ),
        )
    }
}

impl Handler<CommandSet> for Coordinator {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: CommandSet, ctx: &mut Context<Self>) -> Self::Result {
        ctx.set_mailbox_capacity(self.peer_addrs.len() * 10 + 5);

        let mut outgoing_peer_controls: HashMap<(Ipv4Addr, u16), Vec<PeerControl>> = HashMap::new();
        let mut outgoing_peer_changes: HashMap<(Ipv4Addr, u16), Vec<PeerChange>> = HashMap::new();

        for envelope in msg.peer_control.into_iter() {
            for msg in envelope.messages {
                outgoing_peer_controls
                    .entry((envelope.ip_address, envelope.port))
                    .or_default()
                    .push(msg);
            }
        }

        for peer_change_envelope in msg.peer_change.into_iter() {
            outgoing_peer_changes
                .entry((peer_change_envelope.ip_address, peer_change_envelope.port))
                .or_default()
                .push(peer_change_envelope.peer_change);
        }

        let mut peer_control_vec = Vec::new();
        for ((ip_address, port), messages) in outgoing_peer_controls.into_iter() {
            match self.peer_addrs.get(&(ip_address, port)) {
                Some(peer_addr) => peer_control_vec.push(peer_addr.send(PeerControlEnvelope {
                    ip_address,
                    port,
                    messages,
                })),
                None => drop(messages),
            }
        }

        let mut commander_send_vec = Vec::new();
        for ((ip_address, port), msgs) in outgoing_peer_changes.into_iter() {
            commander_send_vec.push(
                self.commander_addr.send(CommanderMsgEnvelope {
                    ip_address,
                    port,
                    msgs: msgs
                        .into_iter()
                        .map(|peer_change| CommanderMsg::PeerChange { peer_change })
                        .collect(),
                }),
            );
        }

        let mut filer_futures = Vec::new();
        if !msg.piece_requests_to_us.is_empty() {
            filer_futures.push(self.filer_addr.send(RequestPieceEnvelope {
                request_pieces: msg.piece_requests_to_us,
            }));
        }

        return Box::pin(
            (async move || {
                for peer_control_fut in peer_control_vec.into_iter() {
                    if let Err(err) = peer_control_fut.await {
                        error!("Peer control error: {}", err);
                    }
                }

                for commander_send_fut in commander_send_vec.into_iter() {
                    if let Err(err) = commander_send_fut.await {
                        error!("Commander error: {}", err);
                    }
                }

                for filer_fut in filer_futures {
                    match filer_fut.await {
                        Ok(_command_set) => {
                            todo!("Massive hassle");
                        }
                        Err(err) => {
                            error!("Error between coordinator and filer: {}", err);
                        }
                    }
                }
            })()
            .into_actor(self),
        );
    }
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct FilerResult {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub event: FilerEvent,
}

impl Handler<FilerResult> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: FilerResult, _ctx: &mut Context<Self>) -> Self::Result {
        unimplemented!()
    }
}

impl Handler<PeerDisconnect> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: PeerDisconnect, ctx: &mut Self::Context) -> Self::Result {
        debug!("Removing peer {}:{}", msg.ip_address, msg.port);
        self.peer_addrs.remove(&(msg.ip_address, msg.port));
        ctx.spawn(
            self.commander_addr
                .send(CommanderMsgEnvelope {
                    ip_address: msg.ip_address,
                    port: msg.port,
                    msgs: vec![CommanderMsg::RemovePeer],
                })
                .into_actor(self)
                .map(|result, _, _| {
                    if let Err(err) = result {
                        error!("Error processing peer disconnect: {}", err);
                    }
                }),
        );
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PeerControlGlobal {
    HavePiece { piece_index: u32 },
}
