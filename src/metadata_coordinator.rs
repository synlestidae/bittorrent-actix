use std::{collections::HashMap, net::Ipv4Addr};

use actix::prelude::*;
use log::{debug, trace};

use crate::{
    bittorrent::{Info, Magnet, Metadata},
    event::{ConnectionEvent, EventHandler, Registration},
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
        debug!("Activated metadata coordinator");
        self.register(ctx);
    }
}

impl Handler<ConnectionEvent> for MetadataCoordinator {
    type Result = ();

    fn handle(&mut self, event: ConnectionEvent, _ctx: &mut Self::Context) -> Self::Result {
        if self.metadata.is_some() {
            return;
        }
        let info_hash = match &self.magnet {
            Some(m) => m.info_hash.clone(),
            None => return,
        };
        match event {
            ConnectionEvent::Connected {
                ip_address,
                port,
                peer,
            } => {
                let downloader = MetadataDownloader::new(
                    (ip_address, port),
                    self.event_handler.clone(),
                    info_hash,
                    peer.clone(),
                );
                self.downloaders
                    .insert((ip_address, port), downloader.start());
            }
            ConnectionEvent::Disconnected { ip_address, port } => {
                self.downloaders.remove(&(ip_address, port));
            }
        }
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
        debug!("Registering metadata coordinator");

        let fut = self.event_handler.send(Registration::Connection {
            recipient: ctx.address().recipient(),
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            result.expect("unable to metadata downloader");
        }));
    }
}
