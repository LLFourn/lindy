use warp::{self, http};

#[derive(Clone, Debug)]
pub enum ApiReply<T> {
    Ok(T),
    Err(ErrorMessage),
    Created(String, T),
    NoContent,
}

// use futures::Future;
// impl<T> ApiReply<T> {
//     pub async fn map<U, F: FnOnce(T) -> Fut, Fut: Future<Output = U>>(self, op: F) -> ApiReply<U> {
//         use ApiReply::*;
//         match self {
//             Ok(t) => Ok(op(t).await),
//             Err(e) => Err(e),
//             Created(s, t) => Created(s, op(t).await),
//         }
//     }

//     pub async fn and_then<U, F: FnOnce(T) -> Fut, Fut: Future<Output = ApiReply<U>>>(
//         self,
//         op: F,
//     ) -> ApiReply<U> {
//         use ApiReply::*;
//         match self {
//             Ok(t) | Created(_, t) => op(t).await,
//             Err(e) => Err(e),
//         }
//     }
// }

impl<T: Send + serde::Serialize> warp::Reply for ApiReply<T> {
    fn into_response(self) -> warp::reply::Response {
        match self {
            ApiReply::Ok(value) => {
                let reply = warp::reply::json(&value);
                reply.into_response()
            }
            ApiReply::Err(err) => warp::reply::with_status(
                warp::reply::json(&err),
                http::StatusCode::from_u16(err.code).unwrap(),
            )
            .into_response(),
            ApiReply::Created(location, value) => {
                let reply = warp::reply::json(&value);
                warp::reply::with_header(reply, "Location", location).into_response()
            }
            ApiReply::NoContent => {
                warp::reply::with_status(warp::reply(), http::StatusCode::NO_CONTENT)
                    .into_response()
            }
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ErrorMessage {
    pub code: u16,
    pub error: String,
}

impl ErrorMessage {
    pub fn not_found() -> Self {
        Self::from_status(http::StatusCode::NOT_FOUND)
    }

    pub fn internal_server_error() -> Self {
        Self::from_status(http::StatusCode::INTERNAL_SERVER_ERROR)
    }

    pub fn bad_request() -> Self {
        Self::from_status(http::StatusCode::BAD_REQUEST)
    }

    pub fn from_status(status: http::StatusCode) -> Self {
        Self {
            code: status.as_u16(),
            error: status.canonical_reason().unwrap().into(),
        }
    }

    pub fn with_message<M: Into<String>>(self, message: M) -> Self {
        Self {
            code: self.code,
            error: message.into(),
        }
    }
}
