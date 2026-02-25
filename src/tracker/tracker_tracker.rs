use std::time::Duration;

use actix::{Actor, Addr, AsyncContext, Context};
use actix::{SpawnHandle, WrapFuture};
use log::{debug, error, info};
use std::cmp::min;
use tokio::time::sleep;

use crate::bittorrent::{Magnet, Metadata};
use crate::event::{EventHandler, TrackerUpdateEvent};
use crate::tracker::AskTracker;
use crate::tracker::Tracker;

pub struct TrackerTracker {
    metadata: Option<Metadata>,
    magnet: Option<Magnet>,
    info_hash: [u8; 20],
    handles: Vec<SpawnHandle>,
    event_handler: Addr<EventHandler>,
}

impl Actor for TrackerTracker {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let tracker_addrs = if let Some(metadata) = &self.metadata {
            vec![Tracker::with_metadata(metadata.clone(), self.info_hash.clone()).start()]
        } else if let Some(magnet) = &self.magnet {
            magnet
                .trackers
                .iter()
                .cloned()
                .map(|t| Tracker::with_uri(t, self.info_hash.clone()).start())
                .collect()
        } else {
            unreachable!("should have either a magnet or metadata")
        };

        self.spawn_trackers(tracker_addrs, ctx);
    }
}

impl TrackerTracker {
    pub fn with_metadata(
        info_hash: [u8; 20],
        metadata: Metadata,
        event_handler: Addr<EventHandler>,
    ) -> Self {
        Self {
            metadata: Some(metadata),
            magnet: None,
            info_hash,
            handles: Vec::new(),
            event_handler,
        }
    }

    pub fn with_magnet(magnet: Magnet, event_handler: Addr<EventHandler>) -> Self {
        Self {
            metadata: None,
            info_hash: magnet.info_hash.clone(),
            magnet: Some(magnet),
            handles: Vec::new(),
            event_handler,
        }
    }

    fn spawn_trackers(&mut self, addrs: Vec<Addr<Tracker>>, ctx: &mut <Self as Actor>::Context) {
        const DEFAULT_INTERVAL_SECS: u64 = 30;

        for addr in addrs.into_iter() {
            let event_handler = self.event_handler.clone();

            let handle = ctx.spawn(
                async move {
                    let mut interval: Duration = Duration::from_secs(0);

                    loop {
                        sleep(interval).await;

                        let tracker_response_result = match addr.send(AskTracker {}).await {
                            Ok(t) => t,
                            Err(err) => {
                                info!("Error sending tracker request: {}", err);
                                info!("Ending tracker loop");
                                return;
                            }
                        };

                        let tracker_response = match tracker_response_result {
                            Ok(t) => t,
                            Err(err) => {
                                info!("Error response from tracker: {:?}", err);
                                interval = Duration::from_secs(DEFAULT_INTERVAL_SECS);
                                continue;
                            }
                        };

                        debug!(
                            "Received tracker response with {} peer(s)",
                            tracker_response.peers.len()
                        );

                        if let Some(tracker_interval) = tracker_response.interval {
                            interval = Duration::from_secs(min(
                                tracker_interval as u64,
                                DEFAULT_INTERVAL_SECS,
                            ));
                        }

                        if let Err(err) = event_handler
                            .send(TrackerUpdateEvent {
                                peers: tracker_response.peers,
                            })
                            .await
                        {
                            error!("Error sending tracker update: {:?}", err);
                            info!("Ending tracker loop");
                            return;
                        }
                    }
                }
                .into_actor(self),
            );

            self.handles.push(handle);
        }
    }
}
