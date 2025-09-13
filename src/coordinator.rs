use std::cmp::max;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::time::Duration;

use actix::prelude::*;
use bitvec::vec::BitVec;
use log::{debug, error};
use rand::Rng;
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::commander::{CommandSet, Commander, CommanderMsg, CommanderMsgEnvelope};
use crate::filer::{Filer, FilerEvent};
use crate::metadata::Info;
use crate::peer::{BittorrentMessage, Peer, PeerError};
use crate::tracker::TrackerResponse;

const TIMEOUT_CONNECT_SECS: u64 = 5;

pub struct Coordinator {
    commander_addr: Addr<Commander>,
    peer_addrs: HashMap<(Ipv4Addr, u16), Addr<Peer>>,
    info_hash: [u8; 20],
}

impl Coordinator {
    pub fn new(info: Info, info_hash: [u8; 20], ctx: &mut Context<Self>) -> Self {
        let filer_addr = Filer::new(info.clone(), ctx.address()).start();
        let commander_addr = Commander::new(info.clone(), ctx.address(), filer_addr).start();

        Self {
            commander_addr,
            peer_addrs: HashMap::new(),
            info_hash,
        }
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
            .filter(|(ip_address, port)| !self.peer_addrs.contains_key(&(*ip_address, *port)))
            .collect();

        ctx.set_mailbox_capacity(max(new_peers.len() * 2, 5));

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
                            info_hash,
                            peer_id,
                            tcp_stream,
                            ctx,
                        )
                    });

                    debug!("Initialised peer {}:{}", msg.ip_address, msg.port);
                    actor.peer_addrs.insert((ip_address, port), peer_addr);
                    actor.commander_addr.do_send(CommanderMsgEnvelope {
                        ip_address,
                        port,
                        msgs: vec![CommanderMsg::AddPeer],
                    });

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

        match msg.msg {
            BittorrentMessage::Choke => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::PeerChange {
                    choking: Some(true),
                    interested: None,
                    bitfield: None,
                    have: None,
                }],
            }),
            BittorrentMessage::Unchoke => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::PeerChange {
                    choking: Some(false),
                    interested: None,
                    bitfield: None,
                    have: None,
                }],
            }),
            BittorrentMessage::Interested => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::PeerChange {
                    choking: None,
                    interested: Some(true),
                    bitfield: None,
                    have: None,
                }],
            }),
            BittorrentMessage::NotInterested => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::PeerChange {
                    choking: None,
                    interested: Some(false),
                    bitfield: None,
                    have: None,
                }],
            }),
            BittorrentMessage::Have { piece_index } => {
                self.commander_addr.do_send(CommanderMsgEnvelope {
                    ip_address: msg.ip_address,
                    port: msg.port,
                    msgs: vec![CommanderMsg::PeerChange {
                        choking: None,
                        interested: None,
                        bitfield: None,
                        have: Some(piece_index),
                    }],
                })
            }
            BittorrentMessage::BitField { bitfield } => {
                let bitfield = BitVec::from_vec(bitfield);
                self.commander_addr.do_send(CommanderMsgEnvelope {
                    ip_address: msg.ip_address,
                    port: msg.port,
                    msgs: vec![CommanderMsg::PeerChange {
                        choking: None,
                        interested: None,
                        bitfield: Some(bitfield),
                        have: None,
                    }],
                })
            }
            BittorrentMessage::Piece {
                index,
                begin,
                block,
            } => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::Piece {
                    index,
                    begin,
                    block,
                }],
            }),
            BittorrentMessage::KeepAlive => (),
            BittorrentMessage::Request {
                index,
                begin,
                length,
            } => self.commander_addr.do_send(CommanderMsgEnvelope {
                ip_address: msg.ip_address,
                port: msg.port,
                msgs: vec![CommanderMsg::PieceRequest {
                    index,
                    begin,
                    length,
                }],
            }),
            BittorrentMessage::Cancel { .. } => (),
            BittorrentMessage::Port { .. } => (),
        };
    }
}

impl Handler<CommandSet> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: CommandSet, _ctx: &mut Context<Self>) -> Self::Result {
        for peer_control in msg.peer_control.into_iter() {
            match self
                .peer_addrs
                .get(&(peer_control.ip_address, peer_control.port))
            {
                Some(peer_addr) => {
                    for message in peer_control.messages {
                        peer_addr.do_send(message);
                    }
                }
                None => (),
            }
        }
    }
}

struct RequestedPiece {
    pub index: u32,
    pub begin: u32,
    pub length: u32,
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

    fn handle(&mut self, msg: PeerDisconnect, _ctx: &mut Self::Context) -> Self::Result {
        debug!("Removing peer {}:{}", msg.ip_address, msg.port);
        self.peer_addrs.remove(&(msg.ip_address, msg.port));
        self.commander_addr.do_send(CommanderMsgEnvelope {
            ip_address: msg.ip_address,
            port: msg.port,
            msgs: vec![CommanderMsg::RemovePeer],
        });
    }
}
