use futures::future;
use futures::StreamExt;
use futures::io;
use futures::{Sink, SinkExt, Stream};
use tokio_util::codec::LinesCodecError;
// use std::collections::HashMap;
// use std::sync::RwLock;
use crate::p2p::messages::Message;
use crate::PeerId;
use tokio::net::TcpStream;
use tokio::net::ToSocketAddrs;
use tokio_util::codec::{Framed, LinesCodec};

async fn frame_connection(
    peer_id: std::net::SocketAddr,
    connection: TcpStream,
) -> Result<(
    impl Sink<Message, Error = Error>,
    impl Stream<Item = Message>,
), Error> {
    let framed = Framed::new(connection, LinesCodec::new());
    let (sink, stream) = framed.split();

    let stream = stream
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got error in socket from {}: {}", peer_id, e);
            }
            future::ready(res.is_ok())
        })
        .map(|res| res.unwrap())
        .map(|line| serde_json::from_str::<Message>(&line))
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got invalid JSON from {}: {}", peer_id, e);
            }
            future::ready(res.is_ok())
        })
        .map(|res| res.unwrap());

    let sink = sink
        .with(async move |message: Message| -> Result<String, Error> {
            Ok(serde_json::to_string(&message).unwrap())
        })
        .sink_map_err(Into::into);

    Ok((sink, stream))
}

pub async fn listen<A: ToSocketAddrs>(
    addr: A,
) -> anyhow::Result<
    impl Stream<
        Item = (
            PeerId,
            impl Sink<Message, Error = Error> + Send + 'static,
            impl Stream<Item = Message> + Send + 'static,
        ),
    >,
> {
    let socket = tokio::net::TcpListener::bind(addr).await?;
    let received_connections = socket.filter_map(async move |connection| match connection {
        Ok(connection) => {
            let peer_addr = connection.peer_addr().unwrap();
            info!("framed connection from {:?}", peer_addr);
            match frame_connection(peer_addr, connection).await {
                Ok((sender, receiver)) => {
                    info!("connected to {}", peer_addr);
                    Some((peer_addr, sender, receiver))
                }
                Err(e) => {
                    error!("failed to frame connection to {}: {}", peer_addr, e);
                    None
                }
            }
        }
        Err(e) => {
            warn!("IO error while receiving connection {}", e);
            None
        }
    });

    Ok(received_connections)
}

pub async fn connect(
    peer_id: PeerId,
) -> Result<(
    impl Sink<Message, Error = Error> + Send + 'static,
    impl Stream<Item = Message> + Send + 'static,
), Error> {
    let new_connection = TcpStream::connect(peer_id).await?;
    let (sender, receiver) = frame_connection(peer_id, new_connection).await?;
    Ok((sender, receiver))
}


#[derive(Clone,Debug, thiserror::Error, Copy)]
pub enum Error {
    #[error("{0:?}")]
    Io(io::ErrorKind),
    #[error("attempted to send message that was too long")]
    MessageTooLong,
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e.kind())
    }
}


impl From<LinesCodecError> for Error {
    fn from(e: LinesCodecError) -> Self {
        match e {
            LinesCodecError::Io(e) => e.into(),
            LinesCodecError::MaxLineLengthExceeded => Error::MessageTooLong
        }
    }
}
