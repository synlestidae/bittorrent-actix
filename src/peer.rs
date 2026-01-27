use std::net::Ipv4Addr;

use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use actix::{Actor, AsyncContext, Context, Handler, WrapFuture};
use tokio::io;
use tokio::net::tcp::OwnedWriteHalf;
use tokio_util::codec::FramedRead;

use crate::bittorrent::{BittorrentCodec, BittorrentMessage, DecoderError};
use crate::event::{Event, EventHandler};

use tokio::net::TcpStream;

use log::{debug, error};

enum Connection {
    Init,
    Connected {
        writer: FramedWrite<BittorrentMessage, OwnedWriteHalf, BittorrentCodec>,
    },
    Dead,
}

pub struct Peer {
    ip_address: Ipv4Addr,
    port: u16,
    connection: Connection,
    event_handler: Addr<EventHandler>,
    messages: Vec<BittorrentMessage>,
}

impl Peer {
    fn connect(&mut self, ctx: &mut <Self as Actor>::Context) {
        let addr = (self.ip_address, self.port);

        ctx.spawn(
            TcpStream::connect(addr)
                .into_actor(self)
                .map(move |result, actor, ctx| {
                    let tcp_stream = match result {
                        Ok(tcp_stream) => tcp_stream,
                        Err(err) => {
                            debug!("Connection error: {}", err);

                            actor.die(ctx);

                            return;
                        }
                    };

                    actor.listen(tcp_stream, ctx);
                }),
        );
    }

    fn listen(&mut self, tcp_stream: TcpStream, ctx: &mut Context<Self>) {
        let (read_half, write_half) = tcp_stream.into_split();

        let codec = BittorrentCodec::new();

        ctx.add_stream(FramedRead::new(read_half, codec.clone()));

        let writer = FramedWrite::new(write_half, codec, ctx);
        self.connection = Connection::Connected { writer };

        //let writer = FramedWrite::new(write_half, codec, ctx);
    }

    fn die(&mut self, ctx: &mut Context<Self>) {
        let ip_address = self.ip_address;
        let port = self.port;

        self.connection = Connection::Dead;

        let fut = self
            .event_handler
            .send(Event::PeerDeath { ip_address, port });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            if let Err(err) = result {
                error!("Error while dying: {}", err);
            }
        }));
    }
}

impl Actor for Peer {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.connect(ctx);
        //self.register(ctx);
    }
}

impl Peer {
    pub fn new(addr: (Ipv4Addr, u16), event_handler: Addr<EventHandler>) -> Self {
        let (ip_address, port) = addr;

        Self {
            ip_address,
            port,
            connection: Connection::Init,
            //registered: false,
            event_handler,
            messages: vec![],
        }
    }
}

/*impl Handler<Event> for Peer {
    type Result = ();

    fn handle(&mut self, msg: Event, ctx: &mut Self::Context) -> Self::Result {
        todo!()
    }
}*/

impl StreamHandler<Result<Vec<BittorrentMessage>, DecoderError>> for Peer {
    fn handle(
        &mut self,
        msg_result: Result<Vec<BittorrentMessage>, DecoderError>,
        ctx: &mut Self::Context,
    ) {
        match msg_result {
            Err(err) => {
                todo!("Error receiving message: {}", err);
            }
            Ok(mut msgs) => {
                self.messages.append(&mut msgs);
                ctx.notify(NotifyRecv {});
            }
        }
    }
}

#[derive(Message)]
#[rtype(result = "()")]
struct NotifyRecv {}

impl Handler<NotifyRecv> for Peer {
    type Result = ();

    fn handle(&mut self, _: NotifyRecv, ctx: &mut Context<Self>) -> Self::Result {
        let messages = self.messages.drain(..).collect();

        let fut = self.event_handler.send(Event::PeerRecvMessages {
            ip_address: self.ip_address,
            port: self.port,
            messages,
        });

        ctx.spawn(fut.into_actor(self).map(|result, _, _| {
            if let Err(err) = result {
                panic!("Error notifying messages: {}", err);
            }
        }));
    }
}

impl WriteHandler<io::Error> for Peer {
    // TODO this sort of error should shut the peer down
}
