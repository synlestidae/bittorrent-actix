use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;

use crate::{
    bittorrent::{BittorrentMessage, Info, Magnet, Metadata},
    event::{Event, EventHandler, RegId, Registration},
    metadata_downloader::MetadataDownloader,
};

pub struct MetadataCoordinator {
    event_handler: Addr<EventHandler>,
    downloaders: HashMap<(Ipv4Addr, u16), Addr<MetadataDownloader>>,
    metadata: Option<Metadata>,
    magnet: Option<Magnet>,
    info: Option<Info>,
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
        // I am impressed at how sparse and ugly the syntax in rust can get
        let (ip_address, port, messages) = match (&self.metadata, &self.magnet, &self.info, event) {
            (None, _, None, Event::Metadata { info }) => {
                self.info = Some(info);

                return;
            }
            (Some(metadata), _, _, Event::PeerConnect { .. }) => {
                let fut = self.event_handler.send(Event::Metadata {
                    info: metadata.info.clone(),
                });
                ctx.spawn(fut.into_actor(self).map(|result, _, _| {
                    result.expect("could not send metadata");
                }));

                return;
            }
            (_, _, _, Event::PeerDeath { ip_address, port }) => {
                self.downloaders.remove(&(ip_address, port));

                return;
            }
            (
                _,
                _,
                _,
                Event::PeerRecvMessages {
                    ip_address,
                    port,
                    messages,
                },
            ) => (ip_address, port, messages),
            _ => return,
        };

        // this point means we are processing a handshake
        // moreover, it means we are in need of metadata
        let (info_hash, exact_length) = match self.magnet {
            Some(Magnet {
                info_hash,
                exact_length,
                ..
            }) => (info_hash, exact_length),
            None => todo!("think this state is somehow unreachable"),
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
    pub fn new(
        metadata: Option<Metadata>,
        magnet: Option<Magnet>,
        event_handler: Addr<EventHandler>,
    ) -> Self {
        Self {
            event_handler,
            downloaders: HashMap::new(),
            metadata,
            magnet,
            info: None,
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
