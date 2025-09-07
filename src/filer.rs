use actix::prelude::*;
use log::debug;
use sha1::{Digest, Sha1};

use std::{collections::HashMap, net::Ipv4Addr};

use crate::{
    coordinator::{Coordinator, FilerResult},
    metadata::Info,
};

pub struct Filer {
    info: Info,
    pieces: HashMap<u32, Vec<u8>>,
    coordinator_addr: Addr<Coordinator>,
}

impl Filer {
    pub fn new(info: Info, coordinator_addr: Addr<Coordinator>) -> Self {
        Self {
            info,
            pieces: HashMap::new(),
            coordinator_addr,
        }
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

pub enum FilerEvent {
    WrongSha1 { piece_index: u32 },
    IndexOutOfBounds { piece_index: u32 },
    FilerMsgErr,
    Accepted { piece_index: u32 },
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct WholePiece {
    pub index: u32,
    pub piece: Vec<u8>,
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

impl Handler<WholePiece> for Filer {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);

        return Box::pin(
            async {}
                .into_actor(self)
                .map(move |_, actor, _| {
                    if actor.info.pieces.len() <= msg.index as usize {
                        return FilerResult {
                            ip_address: msg.ip_address,
                            port: msg.port,
                            event: FilerEvent::IndexOutOfBounds {
                                piece_index: msg.index,
                            },
                        };
                    }

                    let expected_sha1 = match actor.pieces.get(&msg.index) {
                        Some(sha1_sum) => sha1_sum,
                        None => {
                            return FilerResult {
                                ip_address: msg.ip_address,
                                port: msg.port,
                                event: FilerEvent::IndexOutOfBounds {
                                    piece_index: msg.index,
                                },
                            };
                        }
                    };
                    let mut digest = Sha1::new();
                    digest.update(&msg.piece);
                    let actual_sha1_digest = digest.finalize();
                    let actual_sha1 = actual_sha1_digest.as_slice();

                    if expected_sha1 != actual_sha1 {
                        debug!(
                            "SHA-1 hash mismatch expected_sha1={:x?}, actual_sha1_sum={:x?}",
                            actual_sha1, expected_sha1
                        );

                        return FilerResult {
                            ip_address: msg.ip_address,
                            port: msg.port,
                            event: FilerEvent::WrongSha1 {
                                piece_index: msg.index,
                            },
                        };
                    }

                    actor.pieces.insert(msg.index, msg.piece);

                    FilerResult {
                        ip_address: msg.ip_address,
                        port: msg.port,
                        event: FilerEvent::Accepted {
                            piece_index: msg.index,
                        },
                    }
                })
                .map(|filer_result, actor, _| {
                    actor.coordinator_addr.do_send(filer_result);
                }),
        );
    }
}
