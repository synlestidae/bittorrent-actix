use actix::prelude::*;
use log::{debug, error};
use sha1::{Digest, Sha1};

use std::{collections::HashMap, net::Ipv4Addr};

use crate::metadata::{Info, ModeFile};

pub struct Filer {
    info: Info,
    pieces: HashMap<u32, Vec<u8>>,
}

impl Filer {
    pub fn new(info: Info) -> Self {
        Self {
            info,
            pieces: HashMap::new(),
        }
    }
}

impl Actor for Filer {
    type Context = Context<Self>;
}

#[derive(Message)]
#[rtype(result = "FilerResult")]
pub struct WholePiece {
    pub index: u32,
    pub piece: Vec<u8>,
    pub ip_address: Ipv4Addr,
    pub port: u16,
}

pub enum FilerError {
    WrongSha1,
    IndexOutOfBounds,
    FilerMsgErr,
}

pub struct FilerResult {
    pub ip_address: Ipv4Addr,
    pub port: u16,
    pub error: Option<FilerError>,
}

impl Handler<WholePiece> for Filer {
    type Result = ResponseActFuture<Self, FilerResult>;

    fn handle(&mut self, msg: WholePiece, _ctx: &mut Context<Self>) -> Self::Result {
        debug!("Received WholePiece index={}", msg.index);

        return Box::pin(async {}.into_actor(self).map(move |_, actor, _| {
            if actor.info.pieces.len() <= msg.index as usize {
                return FilerResult {
                    ip_address: msg.ip_address,
                    port: msg.port,
                    error: Some(FilerError::IndexOutOfBounds),
                };
            }

            let expected_sha1 = match actor.pieces.get(&msg.index) {
                Some(sha1_sum) => sha1_sum,
                None => {
                    return FilerResult {
                        ip_address: msg.ip_address,
                        port: msg.port,
                        error: Some(FilerError::IndexOutOfBounds),
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
                    error: Some(FilerError::WrongSha1),
                };
            }

            actor.pieces.insert(msg.index, msg.piece);

            FilerResult {
                ip_address: msg.ip_address,
                port: msg.port,
                error: None,
            }
        }));
    }
}
