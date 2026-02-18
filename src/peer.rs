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

use log::{debug, error, info, trace};

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

        debug!("Connecting to {}:{}", self.ip_address, self.port);

        ctx.spawn(
            TcpStream::connect(addr)
                .into_actor(self)
                .map(move |result, actor, ctx| {
                    let tcp_stream = match result {
                        Ok(tcp_stream) => {
                            info!("Connected to {}:{}", addr.0, addr.1);
                            tcp_stream
                        }
                        Err(err) => {
                            debug!("Connection error: {}", err);

                            actor.terminate(ctx);

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

        debug!("Connection to {}:{} successful", self.ip_address, self.port);

        let fut = self.event_handler.send(Event::PeerConnect {
            ip_address: self.ip_address,
            port: self.port,
            addr: ctx.address(),
        });
        ctx.spawn(fut.into_actor(self).map(|result, actor, ctx| {
            match result {
                Err(err) => error!("Error with PeerConnect: {}", err),
                Ok(()) => return,
            }

            actor.terminate(ctx); // if errors in match
        }));
    }

    fn terminate(&mut self, ctx: &mut Context<Self>) {
        trace!("Sending termination event for {}:{}", self.ip_address, self.port);

        let fut = self.event_handler.send(Event::PeerDeath { ip_address: self.ip_address, port: self.port });

        ctx.spawn(fut.into_actor(self).map(|_, _, ctx| {
            // don't care about result, time to die
            ctx.terminate();
        }));
    }
}

impl Actor for Peer {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.connect(ctx);
    }

    fn stopped(&mut self, ctx: &mut Self::Context) {
        debug!("Disconnecting from {}:{}", self.ip_address, self.port);
    }
}

impl Peer {
    pub fn new(addr: (Ipv4Addr, u16), event_handler: Addr<EventHandler>) -> Self {
        let (ip_address, port) = addr;

        Self {
            ip_address,
            port,
            connection: Connection::Init,
            event_handler,
            messages: vec![],
        }
    }
}

impl StreamHandler<Result<Vec<BittorrentMessage>, DecoderError>> for Peer {
    fn handle(
        &mut self,
        msg_result: Result<Vec<BittorrentMessage>, DecoderError>,
        ctx: &mut Self::Context,
    ) {
        match msg_result {
            Err(err) => {
                debug!("Error receiving message: {}", err);
                self.terminate(ctx);
            }
            Ok(mut msgs) => {
                debug!("Received {} message(s)", msgs.len());
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
        let messages: Vec<_> = self.messages.drain(..).collect();

        debug!("Received {} message(s)", messages.len());
        for msg in messages.iter() {
            trace!(
                "Received {}",
                match msg {
                    BittorrentMessage::Piece { index, begin, .. } =>
                        format!("Piece {{ index: {}, begin: {} }}", index, begin),
                    msg => format!("{:?}", msg),
                }
            );
        }

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

#[derive(Debug, Message)]
#[rtype(result = "Result<(), PeerError>")]
pub struct SendMessages {
    pub messages: Vec<BittorrentMessage>,
}

#[derive(Debug)]
pub enum PeerError {
    NotConnected,
}

impl Handler<SendMessages> for Peer {
    type Result = Result<(), PeerError>;

    fn handle(&mut self, msg: SendMessages, _ctx: &mut Self::Context) -> Self::Result {
        if let Connection::Connected { writer } = &mut self.connection {
            trace!(
                "Sending {} message(s) to {}:{}",
                msg.messages.len(),
                self.ip_address,
                self.port
            );

            for msg in msg.messages {
                writer.write(msg);
            }

            return Ok(());
        }

        Err(PeerError::NotConnected)
    }
}
