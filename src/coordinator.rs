use std::cmp::max;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use log::{debug, error, warn};
use rand::Rng;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::bittorrent::{BittorrentMessage, Info};
use crate::filer::{Filer, RequestPiece, RequestPieces, WholePiece};
use crate::peer::{Peer, PeerError, SendMessages};
use crate::tracker::TrackerResponse;

use crate::commander::Command;
use crate::commander::Commander;
use crate::commander::CommanderRequest;

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

    fn send_messages(
        &mut self,
        messages: HashMap<(Ipv4Addr, u16), Vec<BittorrentMessage>>,
    ) -> ResponseActFuture<Self, ()> {
        let peer_addrs = self.peer_addrs.clone();
        Box::pin(
            async move {
                for ((ip_address, port), messages) in messages {
                    match peer_addrs.get(&(ip_address, port)) {
                        Some(peer_addr) => {
                            peer_addr
                                .send(SendMessages { messages })
                                .await
                                .unwrap()
                                .unwrap(); // TODO
                        }
                        _ => unimplemented!(),
                    }
                }
            }
            .into_actor(self),
        )
    }

    fn handle_piece_requests_from_them(
        &mut self,
        request_pieces: RequestPieces,
    ) -> ResponseActFuture<Self, ()> {
        let filer_addr = self.filer_addr.clone();
        let peer_addrs = self.peer_addrs.clone();

        Box::pin(
            async move {
                let messages_to_send = filer_addr.send(request_pieces).await.unwrap().unwrap();
                for (ip_address, port, message) in messages_to_send.into_iter() {
                    match peer_addrs.get(&(ip_address, port)) {
                        Some(peer_addr) => peer_addr
                            .send(SendMessages {
                                messages: vec![message],
                            })
                            .await
                            .unwrap()
                            .unwrap(),
                        None => warn!("Peer {}:{} is missing", ip_address, port),
                    }
                }
            }
            .into_actor(self),
        )
    }

    fn store_whole_pieces(&mut self, whole_pieces: Vec<WholePiece>) -> ResponseActFuture<Self, ()> {
        let filer_addr = self.filer_addr.clone();
        return Box::pin(
            async move {
                for whole_piece in whole_pieces.into_iter() {
                    filer_addr.send(whole_piece).await.unwrap().unwrap();
                }
            }
            .into_actor(self),
        );
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
                            .send(CommanderRequest::AddPeer { ip_address, port })
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

#[derive(Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct RecvMessage {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub messages: Vec<BittorrentMessage>,
}

impl Handler<RecvMessage> for Coordinator {
    type Result = ResponseActFuture<Self, Result<(), PeerError>>;

    fn handle(&mut self, msg: RecvMessage, ctx: &mut Context<Self>) -> Self::Result {
        // forward to the coordinator
        // apply commands
        let request = CommanderRequest::RecvMessages {
            ip_address: msg.ip_address,
            port: msg.port,
            messages: msg.messages,
        };
        let commander_addr = self.commander_addr.clone();

        // TODO Execute commands by scheduling N futures
        // And combine them into one future
        return Box::pin(commander_addr.send(request).into_actor(self).map(
            move |result, actor, ctx| {
                // OK this gets a little complicated, but we can handle it.
                // First of all, we need to send those messages out. AND IN ORDER.
                //
                // Therefore group all messages by peer address
                // Then for each peer, send() that batch of messages
                // We do not need anything by way of response
                //
                // For the piece requests, first request the filer for the piece data
                // Then send() the piece requests as a batch of messages
                //
                // Finally, when we receive a whole piece, forward to the filer.
                // We do not need anything by way of response from filer
                //
                let commands = result.unwrap();

                let mut peer_msgs: HashMap<(Ipv4Addr, u16), Vec<_>> = HashMap::new();

                let mut request_piece_envelope = RequestPieces {
                    request_pieces: Vec::new(),
                };
                let mut whole_pieces = Vec::new();

                commands.into_iter().for_each(|command| match command {
                    Command::SendPeerMessage {
                        ip_address,
                        port,
                        message,
                    } => {
                        peer_msgs
                            .entry((ip_address, port))
                            .or_default()
                            .push(message);
                    }
                    Command::PieceRequestFromThem {
                        ip_address,
                        port,
                        request,
                    } => request_piece_envelope.request_pieces.push(RequestPiece {
                        ip_address,
                        port,
                        index: request.index,
                        begin: request.begin,
                        length: request.length,
                    }),
                    Command::WholePiece {
                        ip_address: _,
                        port: _,
                        whole_piece,
                    } => {
                        whole_pieces.push(whole_piece);
                    }
                });

                // Now all the messages are organised

                // Each of these returns one future, all of whom can be spawned.
                // The order of execution does not matter
                ctx.spawn(actor.send_messages(peer_msgs));
                ctx.spawn(actor.handle_piece_requests_from_them(request_piece_envelope));
                ctx.spawn(actor.store_whole_pieces(whole_pieces));

                Ok(())
            },
        ));
    }
}

impl Handler<PeerDisconnect> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: PeerDisconnect, ctx: &mut Self::Context) -> Self::Result {
        debug!("Removing peer {}:{}", msg.ip_address, msg.port);
        self.peer_addrs.remove(&(msg.ip_address, msg.port));
        ctx.spawn(
            self.commander_addr
                .send(CommanderRequest::RemovePeer {
                    ip_address: msg.ip_address,
                    port: msg.port,
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
