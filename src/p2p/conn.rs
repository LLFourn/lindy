use futures::future;
use futures::io;
use futures::StreamExt;
use futures::TryFutureExt;
use futures::{Sink, SinkExt, Stream};
use std::fmt;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio_util::codec::LinesCodecError;
// use std::collections::HashMap;
// use std::sync::RwLock;
use crate::p2p::messages::Message;
use crate::PeerId;
use secp256kfun::XOnly;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::net::ToSocketAddrs;
use tokio_util::codec::{Framed, LinesCodec};

#[derive(Error, Debug, Clone, Copy)]
pub enum HandshakeError {
    #[error("remote key was invalid")]
    InvalidKey,
    #[error("handshake has an io error of kind {0:?}")]
    Io(io::ErrorKind),
}

impl From<io::Error> for HandshakeError {
    fn from(e: io::Error) -> Self {
        HandshakeError::Io(e.kind())
    }
}

#[derive(Clone, Debug, Copy)]
pub struct Peer {
    pub socket_addr: std::net::SocketAddr,
    pub remote_key: XOnly,
}

impl Peer {
    pub fn id(&self) -> PeerId {
        self.socket_addr
    }
}

impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", &self.remote_key, &self.socket_addr)
    }
}

async fn recv_handshake(
    mut connection: TcpStream,
    point: &XOnly,
) -> Result<(Peer, TcpStream), HandshakeError> {
    connection.write_all(point.as_bytes()).await?;
    let mut xonly = [0u8; 32];
    connection.read_exact(&mut xonly).await?;
    let remote_key = XOnly::from_bytes(xonly).ok_or(HandshakeError::InvalidKey)?;

    let peer = Peer {
        socket_addr: connection.peer_addr()?,
        remote_key,
    };

    Ok((peer, connection))
}

async fn send_handshake(
    mut connection: TcpStream,
    point: &XOnly,
) -> Result<(Peer, TcpStream), HandshakeError> {
    let mut xonly = [0u8; 32];
    connection.read_exact(&mut xonly).await?;
    let remote_key = XOnly::from_bytes(xonly).ok_or(HandshakeError::InvalidKey)?;
    connection.write_all(point.as_bytes()).await?;

    let peer = Peer {
        socket_addr: connection.peer_addr()?,
        remote_key,
    };

    Ok((peer, connection))
}

async fn frame_connection(
    peer: Peer,
    connection: TcpStream,
) -> Result<
    (
        impl Sink<Message, Error = Error>,
        impl Stream<Item = Message>,
    ),
    Error,
> {
    let framed = Framed::new(connection, LinesCodec::new());
    let (sink, stream) = framed.split();

    let stream = stream
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got error in socket from {}: {}", peer, e);
            }
            future::ready(res.is_ok())
        })
        .map(|res| res.unwrap())
        .map(|line| serde_json::from_str::<Message>(&line))
        .take_while(move |res| {
            if let Err(e) = res {
                error!("got invalid JSON from {}: {}", peer, e);
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
    local_key: XOnly,
    addr: A,
) -> anyhow::Result<
    impl Stream<
        Item = (
            Peer,
            impl Sink<Message, Error = Error> + Send + 'static,
            impl Stream<Item = Message> + Send + 'static,
        ),
    >,
> {
    let socket = tokio::net::TcpListener::bind(addr).await?;
    let received_connections = socket.filter_map(async move |connection| match connection {
        Ok(connection) => {
            let peer_addr = connection.peer_addr().ok()?;
            let (peer, connection) = recv_handshake(connection, &local_key)
                .inspect_err(|e| error!("error during handshake with {}: {}", peer_addr, e))
                .await
                .ok()?;
            info!("framed connection from {:?}", peer);
            match frame_connection(peer, connection).await {
                Ok((sender, receiver)) => {
                    info!("connected to {}", peer);
                    Some((peer, sender, receiver))
                }
                Err(e) => {
                    error!("failed to frame connection to {}: {}", peer, e);
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

#[derive(Clone, Debug, Error, Copy)]
pub enum Error {
    #[error("error during handshake: {0}")]
    Handshake(#[from] HandshakeError),
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
            LinesCodecError::MaxLineLengthExceeded => Error::MessageTooLong,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Connector {
    pub local_key: XOnly,
}


impl Connector {
    pub async fn connect(
        self,
        peer_id: PeerId,
    ) -> Result<
            (
                Peer,
                impl Sink<Message, Error = Error> + Send + 'static,
                impl Stream<Item = Message> + Send + 'static,
            ),
        Error,
        > {
        let new_connection = TcpStream::connect(peer_id).await?;
        let (peer, connection) = send_handshake(new_connection, &self.local_key).await?;
        let (sender, receiver) = frame_connection(peer, connection).await?;
        Ok((peer, sender, receiver))
    }
}
