//! The HTTP transport, over a real socket and with a hand-written request: the crate does not
//! depend on an HTTP client, and the point here is what goes on the wire.

#![cfg(feature = "http")]

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use test_provider::{Behavior, Provider};

const REQUEST: &str = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}"#;

/// One request, one answer: enough HTTP for a test, and no client dependency.
///
/// The body is read by `Content-Length` rather than to end of stream: a keep-alive connection stays
/// open, so reading until EOF hangs instead of failing.
fn post(url: &str, body: &str) -> (u16, String) {
    let address = url
        .trim_start_matches("http://")
        .split('/')
        .next()
        .expect("address");
    let mut stream = TcpStream::connect(address).expect("connect to the test provider");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let request = format!(
        "POST /v1/chat/completions HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).expect("write");

    let mut response: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let (head, tail) = loop {
        if let Some(at) = find(&response, b"\r\n\r\n") {
            break (response[..at].to_vec(), response[at + 4..].to_vec());
        }
        let read = stream.read(&mut buffer).expect("read the response head");
        assert_ne!(read, 0, "the connection closed before the headers ended");
        response.extend_from_slice(&buffer[..read]);
    };

    let length: usize = String::from_utf8_lossy(&head)
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .expect("content-length");
    let mut body = tail;
    while body.len() < length {
        let read = stream.read(&mut buffer).expect("read the response body");
        assert_ne!(read, 0, "the connection closed before the body ended");
        body.extend_from_slice(&buffer[..read]);
    }

    let head = String::from_utf8_lossy(&head).into_owned();
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .expect("status line");
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_same_script_answers_over_http() {
    let provider = Arc::new(Provider::new(vec![
        Behavior::usage("over the wire", 30, 5),
        Behavior::unavailable(),
    ]));
    let url = test_provider::http::serve(Arc::clone(&provider)).await;

    let (status, body) = post(&url, REQUEST);
    assert_eq!(status, 200);
    let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
    assert_eq!(value["choices"][0]["message"]["content"], "over the wire");
    assert_eq!(value["usage"]["prompt_tokens"], 30);

    let (status, _) = post(&url, REQUEST);
    assert_eq!(status, 503);
    assert_eq!(provider.calls(), 2);
    assert_eq!(provider.models(), vec!["gpt-4o-mini".to_owned(); 2]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_http_transport_echoes_the_model_too() {
    let provider = Arc::new(Provider::always(Behavior::Responds {
        status: 200,
        body: r#"{"choices":[{"message":{"content":"hi"}}]}"#.to_owned(),
    }));
    let url = test_provider::http::serve(provider).await;

    let (status, body) = post(&url, REQUEST);
    assert_eq!(status, 200);
    let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
    assert_eq!(value["model"], "gpt-4o-mini");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stream_arrives_as_sse_chunks() {
    let provider = Arc::new(Provider::always(Behavior::Stream(vec![
        "one".to_owned(),
        "two".to_owned(),
    ])));
    let url = test_provider::http::serve(provider).await;

    let (status, body) = post(&url, REQUEST);
    assert_eq!(status, 200);
    assert_eq!(body, "data: one\n\ndata: two\n\n");
}
