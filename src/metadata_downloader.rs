use std::{
    io::{BufReader, Cursor},
    net::Ipv4Addr,
    time::Duration,
};

use actix::prelude::*;

use log::{debug, error, info, trace};
use sha1::{Digest, Sha1};

use crate::{
    bittorrent::{Bencoding, BittorrentMessage, ExtensionMessage, Info},
    event::{EventHandler, MetadataEvent, RecvMessageEvent, Registration},
    peer::{Peer, SendMessages},
};

pub struct MetadataDownloader {
    ip_address: Ipv4Addr,
    port: u16,
    peer: Addr<Peer>,
    data: Vec<u8>,
    metadata_handshake_sent: bool,
    metadata_handshake_received: bool,
    event_handler: Addr<EventHandler>,
    exact_length: Option<usize>,
    next_piece: usize,
    info_hash: [u8; 20],
    request_made: bool,
}

const BLOCK_SIZE: usize = 16384;

const UT_METADATA: u32 = 3;

impl MetadataDownloader {
    pub fn new(
        addr: (Ipv4Addr, u16),
        event_handler: Addr<EventHandler>,
        info_hash: [u8; 20],
        peer: Addr<Peer>,
    ) -> Self {
        let (ip_address, port) = addr;

        Self {
            ip_address,
            port,
            peer,
            event_handler,
            data: Vec::new(),
            info_hash,
            next_piece: 0,
            request_made: false,
            metadata_handshake_sent: false,
            metadata_handshake_received: false,
            exact_length: None,
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        debug!(
            "registering metadata downloader for {}:{}",
            self.ip_address, self.port
        );

        let fut = self.event_handler.send(Registration::RecvMessage {
            ip_address: self.ip_address,
            port: self.port,
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, actor, ctx| {
            result.expect("unable to metadata downloader");
            actor.schedule(ctx);
        }));
    }

    fn schedule(&self, ctx: &mut Context<Self>) {
        ctx.run_interval(Duration::from_secs(5), |actor, ctx| {
            actor.request(ctx);
        });
    }

    fn try_finalise(&mut self, ctx: &mut Context<Self>) {
        // we assume the length is OK
        // first check the hash
        let mut digest = Sha1::new();
        digest.update(&self.data);
        let info_hash = digest.finalize();

        trace!("finalising metadata");

        if info_hash.to_vec() != self.info_hash.to_vec() {
            info!(
                "Metadata from {}:{} does not match hash",
                self.ip_address, self.port
            );
            return; // TODO log something
        }

        let cursor = Cursor::new(self.data.clone());
        let mut bufreader = BufReader::new(cursor);
        let bencoding = match Bencoding::deserialise(&mut bufreader) {
            Ok(b) => b,
            Err(err) => {
                error!("Error parsing info: {:?}", err);
                self.data = vec![];
                return;
            }
        };
        let info = match Info::parse(bencoding) {
            Ok(i) => i,
            Err(err) => {
                error!("Error parsing info: {:?}", err);
                self.data = vec![];
                return;
            }
        };

        info!("metadata downloaded successfully");

        let fut = self.event_handler.send(MetadataEvent { info });
        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to deliver metadata");
        }));
    }

    fn request(&mut self, ctx: &mut Context<Self>) {
        if !self.metadata_handshake_sent {
            self.metadata_handshake_sent = true;
            let messages = vec![BittorrentMessage::Extension {
                message: ExtensionMessage::Handshake {
                    metadata_size: None,
                    ut_metadata: UT_METADATA,
                },
            }];
            let fut = self.peer.send(SendMessages { messages });
            ctx.spawn(fut.into_actor(self).map(|result, _, _| {
                result
                    .expect("unable to send extension handshake")
                    .expect("unable to send extension handshake");
            }));
        }
        if !self.metadata_handshake_received {
            return;
        }
        if self.request_made {
            return;
        }

        debug!("requesting metadata from {}:{}", self.ip_address, self.port);

        let exact_length = match self.exact_length {
            Some(l) => l,
            None => {
                debug!("do not know exact length");
                return;
            }
        };

        debug!("metadata has exact length {}", exact_length);

        self.request_made = true;

        let mut messages = vec![];
        let mut piece_index: u32 = 0;

        while piece_index as usize * BLOCK_SIZE < exact_length {
            messages.push(BittorrentMessage::Extension {
                message: ExtensionMessage::Request { piece_index },
            });
            piece_index += 1;
        }

        // Send command here
        ctx.spawn(
            self.peer
                .send(SendMessages { messages })
                .into_actor(self)
                .map(|result, _, _| {
                    result
                        .expect("unable to send peer messages")
                        .expect("unable to send peer messages");
                }),
        );
    }
}

impl Actor for MetadataDownloader {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
        self.schedule(ctx);
    }
}

impl Handler<RecvMessageEvent> for MetadataDownloader {
    type Result = ();

    fn handle(&mut self, msg: RecvMessageEvent, ctx: &mut Self::Context) -> Self::Result {
        let messages = msg.messages;

        messages.iter().for_each(|m| match m {
            BittorrentMessage::Extension {
                message:
                    ExtensionMessage::Handshake {
                        metadata_size,
                        ut_metadata,
                    },
            } => {
                trace!(
                    "Received extension handshake metadata_size={:?}, ut_metadata={:?}",
                    metadata_size, ut_metadata
                );
                self.exact_length = metadata_size.map(|m| m as usize);
            }
            _ => (),
        });

        let mut metadatas: Vec<_> = messages
            .into_iter()
            .filter_map(|m| {
                match m {
                    BittorrentMessage::Extension {
                        message:
                            ExtensionMessage::Data {
                                piece_index,
                                total_size,
                                data,
                            },
                    } => Some((piece_index, total_size, data)), // TODO sort this out
                    _ => return None,
                }
            })
            .collect();

        metadatas.sort_by_key(|(piece_index, _, _)| *piece_index);

        for (piece_index, _, mut data) in metadatas.into_iter() {
            if self.next_piece == piece_index as usize {
                self.data.append(&mut data);
                self.next_piece += 1;
            }
        }

        let exact_length = match self.exact_length {
            Some(l) => l,
            None => return,
        };

        if self.data.len() == exact_length {
            self.try_finalise(ctx);
        }

        if self.data.len() >= exact_length {
            self.next_piece = 0;
            self.data = vec![];
        }
    }
}
