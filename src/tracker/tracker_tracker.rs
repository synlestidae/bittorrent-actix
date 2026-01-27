use std::time::Duration;

use actix::{Actor, Addr, AsyncContext, Context, Recipient};
use actix::{SpawnHandle, WrapFuture};
use log::{error, info};
use std::cmp::min;
use tokio::time::sleep;

use crate::event::Event;
use crate::tracker::AskTracker;
use crate::tracker::{MetadataSource, Tracker};

pub struct TrackerTracker {
    metadata_source: MetadataSource,
    handles: Vec<SpawnHandle>,
    event_handler: Recipient<Event>,
}

impl Actor for TrackerTracker {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        let tracker_addrs = match self.metadata_source {
            MetadataSource::Magnet {
                ref magnet,
                ref info_hash,
            } => magnet
                .trackers
                .iter()
                .cloned()
                .map(|t| Tracker::with_uri(t, info_hash.clone()).start())
                .collect(),
            MetadataSource::Metadata {
                ref metadata,
                ref info_hash,
            } => {
                let metadata = metadata.clone();

                vec![Tracker::with_metadata(metadata, info_hash.clone()).start()]
            }
        };

        self.spawn_trackers(tracker_addrs, ctx);
    }
}

impl TrackerTracker {
    pub fn new(metadata_source: MetadataSource, event_handler: Recipient<Event>) -> Self {
        Self {
            metadata_source,
            event_handler,
            handles: vec![],
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

                        if let Some(tracker_interval) = tracker_response.interval {
                            interval = Duration::from_secs(min(
                                tracker_interval as u64,
                                DEFAULT_INTERVAL_SECS,
                            ));
                        }

                        if let Err(err) = event_handler
                            .send(Event::HttpTrackerResponse {
                                response: tracker_response,
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
