use std::{collections::HashMap, net::Ipv4Addr};

use actix::{WeakRecipient, prelude::*};
use log::{error, trace};
use tokio::task::JoinSet;

use crate::event::{
    Registration, TrackerUpdateEvent,
    event::{ConnectionEvent, MetadataEvent, RecvMessageEvent},
};

pub struct EventHandler {
    connection_recipients: Vec<WeakRecipient<ConnectionEvent>>,
    recv_message_recipients: HashMap<(Ipv4Addr, u16), WeakRecipient<RecvMessageEvent>>,
    tracker_update_recipients: Vec<WeakRecipient<TrackerUpdateEvent>>,
    metadata_recipients: Vec<WeakRecipient<MetadataEvent>>,
}

impl EventHandler {
    pub fn new() -> Self {
        Self {
            connection_recipients: vec![],
            recv_message_recipients: HashMap::new(),
            tracker_update_recipients: vec![],
            metadata_recipients: vec![],
        }
    }
}

impl Actor for EventHandler {
    type Context = Context<Self>;
}

impl Handler<Registration> for EventHandler {
    type Result = ();

    fn handle(&mut self, msg: Registration, _ctx: &mut Self::Context) -> Self::Result {
        match msg {
            Registration::Connection { recipient } => {
                self.connection_recipients.push(recipient.downgrade());
            }
            Registration::RecvMessage {
                ip_address,
                port,
                recipient,
            } => {
                self.recv_message_recipients
                    .insert((ip_address, port), recipient.downgrade());
            }
            Registration::Metadata { recipient } => {
                self.metadata_recipients.push(recipient.downgrade());
            }
        };
    }
}

impl Handler<ConnectionEvent> for EventHandler {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: ConnectionEvent, _ctx: &mut Self::Context) -> Self::Result {
        let mut joinset = JoinSet::new();
        self.connection_recipients.retain(|w| {
            let recipient = match w.upgrade() {
                None => return false,
                Some(r) => r,
            };

            joinset.spawn(recipient.send(msg.clone()));

            true
        });

        return Box::pin(joinset.join_all().into_actor(self).map(|result, _, _| {
            trace!("Dispatched {} connection event(s)", result.len());
        }));
    }
}

impl Handler<RecvMessageEvent> for EventHandler {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: RecvMessageEvent, _ctx: &mut Self::Context) -> Self::Result {
        let recipient = match self
            .recv_message_recipients
            .get(&(msg.ip_address, msg.port))
            .map(|w| w.upgrade())
        {
            Some(Some(r)) => Some(r),
            _ => None,
        };
        if recipient.is_none() {
            self.recv_message_recipients
                .remove(&(msg.ip_address, msg.port));
            return Box::pin(async {}.into_actor(self));
        }

        let r = recipient.unwrap();

        return Box::pin(r.send(msg).into_actor(self).map(|result, _, _| {
            if result.is_err() {
                error!("error sending recv message event: {}", result.unwrap_err());
            }
        }));
    }
}

impl Handler<TrackerUpdateEvent> for EventHandler {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: TrackerUpdateEvent, _ctx: &mut Self::Context) -> Self::Result {
        let mut joinset = JoinSet::new();

        self.tracker_update_recipients
            .retain(|w| match w.upgrade() {
                Some(recipient) => {
                    joinset.spawn(recipient.send(msg.clone()));
                    true
                }
                None => false,
            });

        Box::pin(joinset.join_all().into_actor(self).map(|result, _, _| {
            trace!("Sent tracker update to {} listener(s)", result.len());
        }))
    }
}

impl Handler<MetadataEvent> for EventHandler {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: MetadataEvent, _ctx: &mut Self::Context) -> Self::Result {
        let mut joinset = JoinSet::new();

        self.metadata_recipients.retain(|w| match w.upgrade() {
            Some(recipient) => {
                joinset.spawn(recipient.send(msg.clone()));
                true
            }
            None => false,
        });

        Box::pin(joinset.join_all().into_actor(self).map(|result, _, _| {
            trace!("Sent metadata update to {} listener(s)", result.len());
        }))
    }
}
