use std::{
    io::{BufReader, Cursor},
    net::Ipv4Addr,
    time::Duration,
};

use actix::prelude::*;

use log::error;
use sha1::{Digest, Sha1};

use crate::{
    bittorrent::{Bencoding, BittorrentMessage, ExtensionMessage, Info},
    event::{Command, Event, EventHandler, EventPredicate, RegId, Registration},
};

pub struct MetadataDownloader {
    ip_address: Ipv4Addr,
    port: u16,
    event_handler: Addr<EventHandler>,
    data: Vec<u8>,
    exact_length: Option<usize>,
    next_piece: usize,
    info_hash: [u8; 20],
    request_made: bool,
}

const BLOCK_SIZE: usize = 16384;

impl MetadataDownloader {
    pub fn new(
        addr: (Ipv4Addr, u16),
        event_handler: Addr<EventHandler>,
        info_hash: [u8; 20],
        exact_length: Option<usize>,
    ) -> Self {
        let (ip_address, port) = addr;

        Self {
            ip_address,
            port,
            event_handler,
            data: Vec::new(),
            exact_length,
            info_hash,
            next_piece: 0,
            request_made: false,
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        let fut = self.event_handler.send(Registration::Activate {
            id: RegId::next(),
            predicate: Some(EventPredicate {
                peer_address: Some((self.ip_address, self.port)),
                piece_index: None,
                message_type: None,
            }),
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

        if info_hash.to_vec() != self.info_hash.to_vec() {
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

        let fut = self.event_handler.send(Event::Metadata { info });
        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to deliver metadata");
        }));
    }

    fn request(&mut self, ctx: &mut Context<Self>) {
        if self.request_made {
            return;
        }

        let exact_length = match self.exact_length {
            Some(l) => l,
            None => return,
        };

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
            self.event_handler
                .send(Command::PeerSendMessages {
                    ip_address: self.ip_address,
                    port: self.port,
                    messages,
                })
                .into_actor(self)
                .map(|result, _, _| {
                    result.expect("unable to send peer messages");
                }),
        );
    }
}

impl Handler<Event> for MetadataDownloader {
    type Result = ();

    fn handle(&mut self, msg: Event, ctx: &mut Self::Context) -> Self::Result {
        let messages = match msg {
            Event::PeerRecvMessages {
                ip_address,
                port,
                messages,
            } => {
                if (ip_address, port) != (self.ip_address, self.port) {
                    return;
                }

                messages
            }
            _ => return,
        };

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

impl Actor for MetadataDownloader {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
        self.schedule(ctx);
    }
}
