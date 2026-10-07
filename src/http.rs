//! The HTTP transport: the same script, over a real socket.
//!
//! For the code that talks to a provider with an HTTP client — `reqwest`, `hyper`, whatever the
//! consumer uses — a fake that answers in process is not enough. This serves
//! `POST /v1/chat/completions` on a loopback port, with the OpenAI-compatible shape the clients
//! expect, and answers out of the same [`Provider`](crate::Provider).
//!
//! The server is spawned on the caller's runtime and lives until the process ends, which is what a
//! test wants: nothing to shut down, nothing to leak between tests because each one binds its own
//! port.

use std::sync::Arc;

use axum::body::Bytes;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;
use tokio::net::TcpListener;

use crate::{Answer, Provider};

/// Serves `provider` on `127.0.0.1`, on a port the OS picks, and returns the base URL to point a
/// client at (`http://127.0.0.1:PORT/v1`).
///
/// # Panics
///
/// Panics when the port cannot be bound, which in a test is the failure one wants to see.
pub async fn serve(provider: Arc<Provider>) -> String {
    let router = Router::new()
        .route("/v1/chat/completions", post(handle))
        .with_state(provider);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the test provider");
    let address = listener.local_addr().expect("test provider address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{address}/v1")
}

/// The handler: one answer from the script, in the transport shape the script asked for.
async fn handle(
    axum::extract::State(provider): axum::extract::State<Arc<Provider>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    match provider.answer(&body) {
        Answer::Responded { status, body } => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Answer::Streamed { status, chunks } => {
            let body = chunks
                .into_iter()
                .map(|chunk| {
                    if chunk.starts_with("data:") {
                        format!("{chunk}\n\n")
                    } else {
                        format!("data: {chunk}\n\n")
                    }
                })
                .collect::<String>();
            (
                StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
                [(header::CONTENT_TYPE, "text/event-stream")],
                body,
            )
                .into_response()
        }
        // The two failure modes have no HTTP shape: a provider that answers with a status has
        // answered. A consumer that needs them uses the in-process transport, where they are
        // reported as errors instead of being invented here.
        Answer::NotSent | Answer::SentWithoutResponse => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"error":{"message":"this behavior has no HTTP shape: use the in-process transport"}}"#,
        )
            .into_response(),
    }
}
