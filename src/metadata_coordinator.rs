use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;

use crate::{
    bittorrent::BittorrentMessage,
    event::{Event, EventHandler, RegId, Registration},
    metadata_downloader::MetadataDownloader,
    tracker::MetadataSource,
};

pub struct MetadataCoordinator {
    event_handler: Addr<EventHandler>,
    downloaders: HashMap<(Ipv4Addr, u16), Addr<MetadataDownloader>>,
    metadata_source: MetadataSource,
}

impl Actor for MetadataCoordinator {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.register(ctx);
    }
}

impl Handler<Event> for MetadataCoordinator {
    type Result = ();

    fn handle(&mut self, event: Event, ctx: &mut Self::Context) -> Self::Result {
        let (ip_address, port, messages) = match (&self.metadata_source, event) {
            (
                MetadataSource::Magnet {
                    info_hash,
                    magnet: _,
                },
                Event::Metadata { metadata },
            ) => {
                self.metadata_source = MetadataSource::Metadata {
                    info_hash: info_hash.clone(),
                    metadata,
                };

                return;
            }
            (
                MetadataSource::Metadata {
                    info_hash: _,
                    metadata,
                },
                Event::PeerConnect { .. },
            ) => {
                let fut = self.event_handler.send(Event::Metadata {
                    metadata: metadata.clone(),
                });
                ctx.spawn(fut.into_actor(self).map(|result, _, _| {
                    result.expect("could not send metadata");
                }));

                return;
            }
            (_, Event::PeerDeath { ip_address, port }) => {
                self.downloaders.remove(&(ip_address, port));

                return;
            }
            (
                _,
                Event::PeerRecvMessages {
                    ip_address,
                    port,
                    messages,
                },
            ) => (ip_address, port, messages),
            _ => return,
        };

        let (info_hash, exact_length) = match &self.metadata_source {
            MetadataSource::Metadata { .. } => return,
            MetadataSource::Magnet { info_hash, magnet } => (info_hash, magnet.exact_length),
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

        let downloader = MetadataDownloader::new(
            (ip_address, port),
            self.event_handler.clone(),
            info_hash.clone(),
            exact_length,
        );
        self.downloaders
            .insert((ip_address, port), downloader.start());
    }
}

impl MetadataCoordinator {
    pub fn new(metadata_source: MetadataSource, event_handler: Addr<EventHandler>) -> Self {
        Self {
            event_handler,
            downloaders: HashMap::new(),
            metadata_source,
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
