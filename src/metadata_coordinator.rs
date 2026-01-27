use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;

use crate::{
    bittorrent::{BittorrentMessage, Metadata},
    event::{Event, EventHandler, RegId, Registration},
    metadata_downloader::MetadataDownloader,
};

pub struct MetadataCoordinator {
    event_handler: Addr<EventHandler>,
    downloaders: HashMap<(Ipv4Addr, u16), Addr<MetadataDownloader>>,
    metadata: Option<Metadata>,
}

impl Actor for MetadataCoordinator {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}

impl Handler<Event> for MetadataCoordinator {
    type Result = ();

    fn handle(&mut self, msg: Event, _ctx: &mut Self::Context) -> Self::Result {
        let (ip_address, port, messages) = match msg {
            Event::PeerRecvMessages {
                ip_address,
                port,
                messages,
            } => (ip_address, port, messages),
            Event::PeerDeath { ip_address, port } => {
                self.downloaders.remove(&(ip_address, port));
                return;
            }
            Event::Metadata { metadata } => {
                if self.metadata.is_some() {
                    return;
                }

                self.metadata = Some(metadata);
                return;
            }
            _ => return,
        };

        let handshakes: Vec<_> = messages
            .into_iter()
            .filter_map(|m| match m {
                BittorrentMessage::Handshake { handshake } => Some(handshake),
                _ => None,
            })
            .collect();
        if handshakes.len() == 0 {
            return;
        }
        let handshake = &handshakes[0];

        if !handshake.extension() {
            return;
        }

        let downloader = MetadataDownloader::new((ip_address, port), self.event_handler.clone());
        self.downloaders
            .insert((ip_address, port), downloader.start());
    }
}

impl MetadataCoordinator {
    pub fn new(event_handler: Addr<EventHandler>) -> Self {
        Self {
            event_handler,
            downloaders: HashMap::new(),
            metadata: None,
        }
    }

    fn register(&self, ctx: &mut Context<Self>) {
        let fut = self.event_handler.send(Registration::Activate {
            id: RegId::next(),
            predicate: None,
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to metadata downloader");
        }));
    }
}
