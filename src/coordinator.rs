use actix::prelude::*;

#[derive(Default)]
pub struct Coordinator {}

impl Actor for Coordinator {
    type Context = Context<Self>;
}

pub struct TrackerPeer {
    pub peer_id: String,
    pub id: String,
    pub port: u16,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct UpdatePeers {
    pub peers: Vec<TrackerPeer>,
}

impl Handler<UpdatePeers> for Coordinator {
    type Result = ();

    fn handle(&mut self, msg: UpdatePeers, _ctx: &mut Context<Self>) -> Self::Result {
        // add new peers to dictionary
        // initialise actors for them
        // init the peers if we don't have enough
        unimplemented!()
    }
}
