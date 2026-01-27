use std::net::Ipv4Addr;

use actix::prelude::*;

use crate::{bittorrent::{BittorrentMessage, ExtensionMessage}, event::{Event, EventHandler, EventPredicate, RegId, Registration}};

pub struct MetadataDownloader {
    ip_address: Ipv4Addr,
    port: u16,
    event_handler: Addr<EventHandler>,
    data: Vec<u8>,
}

impl MetadataDownloader {
    pub fn new(addr: (Ipv4Addr, u16), event_handler: Addr<EventHandler>) -> Self {
        let (ip_address, port) = addr;

        Self {
            ip_address,
            port,
            event_handler,
            data: Vec::new(),
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

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to metadata downloader");
        }));
    }
}

impl Handler<Event> for MetadataDownloader {
    type Result = ();

    fn handle(&mut self, msg: Event, ctx: &mut Self::Context) -> Self::Result {
        let messages = match msg {
            Event::PeerRecvMessages { ip_address, port, messages } => {
                if (ip_address, port) != (self.ip_address, self.port) {
                    return;
                }

                messages
            },
            _ => return,
        };

        let metadatas = messages.into_iter().filter_map(|m| {
            match m {
                BittorrentMessage::ExtensionMessage { message: ExtensionMessage::Data { piece_index, total_size, data } } // TODO sort this out 
            }
        });
    }
}

impl Actor for MetadataDownloader {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}
