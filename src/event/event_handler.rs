use actix::{WeakRecipient, prelude::*};
use tokio::task::JoinSet;

use crate::event::{Event, event::Registration};

pub struct EventHandler {
    recipients: Vec<WeakRecipient<Event>>,
}

impl EventHandler {
    pub fn new() -> Self {
        Self { recipients: vec![] }
    }
}

impl Actor for EventHandler {
    type Context = Context<Self>;
}

impl Handler<Event> for EventHandler {
    type Result = ResponseActFuture<Self, ()>;

    fn handle(&mut self, msg: Event, _ctx: &mut Self::Context) -> Self::Result {
        let mut joinset = JoinSet::new();

        self.recipients.retain(|weak_recipient| {
            // doing two things here - sending message and clearing out
            // recipients who are dropped (couldn't upgrade)
            if let Some(r) = weak_recipient.upgrade() {
                joinset.spawn(r.send(msg.clone()));
                return true;
            }

            false
        });

        Box::pin(joinset.join_all().into_actor(self).map(|results, _, _| {
            for result in results.into_iter() {
                if let Err(err) = result {
                    panic!("Error in event handler: {}", err);
                }
            }
        }))
    }
}

impl Handler<Registration> for EventHandler {
    type Result = ();

    fn handle(&mut self, msg: Registration, _ctx: &mut Self::Context) -> Self::Result {
        if let Registration::Activate {
            id: _,
            predicate: _,
            recipient,
        } = msg
        {
            let weak_recipient = WeakRecipient::from(recipient);
            self.recipients.push(weak_recipient);
        }
    }
}
