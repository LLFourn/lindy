use std::convert::Infallible;

use futures::channel::mpsc;
use futures::channel::oneshot;
use futures::SinkExt;
use warp::http::StatusCode;
use warp::Filter;
use warp::Rejection;

use crate::p2p::event::Event;
use crate::p2p::messages::Message;
use crate::PeerId;

#[derive(serde::Deserialize, serde::Serialize)]
struct ChannelCreate {
    hello: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NewPeerReq {
    id: PeerId,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct NewPeerRes {
    id: PeerId,
    location: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Error {
    description: String,
}

impl<E: std::error::Error> From<E> for Error {
    fn from(e: E) -> Self {
        Error {
            description: format!("{}", e),
        }
    }
}

pub fn routes(
    event_sender: mpsc::Sender<Event>,
) -> impl Filter<Extract = impl warp::Reply, Error = Rejection> + Clone {
    let new_peer = warp::post()
        .and(add_event_sender(event_sender.clone()))
        .and(warp::path("peers"))
        .and(warp::body::json::<NewPeerReq>())
        .and_then(connect);

    let update_peer = warp::put()
        .and(add_event_sender(event_sender.clone()))
        .and(warp::path("peers"))
        .and(warp::path::param())
        .and(warp::body::json::<Message>())
        .and_then(send_message);

    new_peer.or(update_peer)
}

fn add_event_sender(
    sender: mpsc::Sender<Event>,
) -> impl Filter<Extract = (mpsc::Sender<Event>,), Error = Infallible> + Clone {
    warp::any().map(move || sender.clone())
}

async fn connect(
    mut sender: mpsc::Sender<Event>,
    new_peer: NewPeerReq,
) -> Result<impl warp::Reply, Infallible> {
    let (notifier, on_connect) = oneshot::channel();
    let _ = sender
        .send(Event::OutgoingConnectionReq {
            peer_id: new_peer.id,
            notify: Some(notifier),
        })
        .await;

    Ok(on_connect
        .await
        .map(|res| match res {
            Ok(_) => warp::reply::with_status(
                warp::reply::json(&NewPeerRes {
                    id: new_peer.id,
                    location: format!("/{}", new_peer.id),
                }),
                StatusCode::CREATED,
            ),
            Err(e) => warp::reply::with_status(
                warp::reply::json(&Error::from(e)),
                StatusCode::FAILED_DEPENDENCY,
            ),
        })
        //TOOD: Fix unwrap
        .unwrap())
}

async fn send_message(
    mut sender: mpsc::Sender<Event>,
    peer_id: PeerId,
    message: Message,
) -> Result<impl warp::Reply, Infallible> {
    let (notifier, on_sent) = oneshot::channel();
    let _ = sender
        .send(Event::SendMessageReq {
            peer_id,
            message: (message, Some(notifier)),
        })
        .await;

    Ok(on_sent
        .await
        .map(|_| warp::reply::with_status(warp::reply(), StatusCode::NO_CONTENT))
        .unwrap_or_else(|_e| {
            warp::reply::with_status(warp::reply(), StatusCode::FAILED_DEPENDENCY)
        }))
}
