//! A provider that does exactly what it is told, and counts how many times.
//!
//! Every project that tests against an LLM ends up writing this: a fake provider whose answer is
//! scripted, because a test that hits a real API fails for reasons that have nothing to do with the
//! code under test. The fake then grows, and the copies of it drift — which is not hypothetical:
//! the HTTP copy in `llmlab` had to learn to echo the model, because a real provider does and
//! without it the agent's recorded trace says `unknown` and its replay stops matching. The other
//! copies never learned it.
//!
//! So the behaviours live here, as data, and the transport is a detail:
//!
//! - [`Provider::answer`] is the in-process transport: give it a request body, get back what the
//!   script says, with the request recorded and the call counted;
//! - [`serve`](http::serve) (feature `http`) is the HTTP transport: a real server on a loopback
//!   port, for the code that talks over the wire.
//!
//! What is *not* here is the glue to a particular trait. `llmgateway`'s `Upstream` and its
//! `TransportError` belong to `llmgateway`, and a test crate that owned them would have to be
//! released in lockstep with the library. Each consumer writes its own twenty-line adapter onto
//! [`Answer`], and that adapter is the only part that cannot be shared.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// What the provider is told to do, one entry per call. The last entry repeats forever, so
/// "always unavailable" is one entry and not a number.
#[derive(Debug, Clone)]
pub enum Behavior {
    /// A `200` whose body carries this completion and this usage, with the requested model
    /// echoed into it like in every other JSON response.
    Ok {
        /// The text of the completion.
        content: String,
        /// Prompt tokens the provider reports.
        prompt_tokens: u64,
        /// Completion tokens the provider reports.
        completion_tokens: u64,
    },
    /// Any status and body, forwarded as they are. A JSON body gets the requested model echoed
    /// into it, exactly as a real provider echoes it.
    Responds {
        /// The HTTP status.
        status: u16,
        /// The raw body.
        body: String,
    },
    /// Nothing comes back, and the request **did not go out**: a connection refused, a DNS
    /// failure, a timeout before anything was written.
    NotSent,
    /// Nothing comes back, and the request **did go out**: retrying may charge twice, which is the
    /// case a failover policy has to get right.
    SentWithoutResponse,
    /// A `200` delivered in chunks, never accumulated into one body.
    Stream(Vec<String>),
}

impl Behavior {
    /// A `200` with a completion and no usage worth metering.
    #[must_use]
    pub fn ok(content: impl Into<String>) -> Self {
        Self::Ok {
            content: content.into(),
            prompt_tokens: 1_000,
            completion_tokens: 500,
        }
    }

    /// A `200` with a completion and the usage the caller wants metered.
    #[must_use]
    pub fn usage(content: impl Into<String>, prompt_tokens: u64, completion_tokens: u64) -> Self {
        Self::Ok {
            content: content.into(),
            prompt_tokens,
            completion_tokens,
        }
    }

    /// A `503` with the error a service under stress produces.
    #[must_use]
    pub fn unavailable() -> Self {
        Self::Responds {
            status: 503,
            body: r#"{"error":{"message":"service unavailable"}}"#.to_owned(),
        }
    }
}

/// What the provider answers. The consumer maps this onto its own transport types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A status and a complete body.
    Responded {
        /// The HTTP status to send.
        status: u16,
        /// The body to send.
        body: String,
    },
    /// A status and the chunks of a streamed body.
    Streamed {
        /// The HTTP status to send.
        status: u16,
        /// The chunks, in order, as the caller should forward them.
        chunks: Vec<String>,
    },
    /// The request never left. The consumer reports this as its own "not sent" error.
    NotSent,
    /// The request left and nothing came back. The consumer reports this as its own "maybe sent"
    /// error, and owns the consequence.
    SentWithoutResponse,
}

/// One call the provider received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The body, as it arrived.
    pub body: String,
    /// The model the body asked for, when it named one.
    pub model: Option<String>,
}

/// A provider with a script, a call counter and the requests it received.
#[derive(Debug)]
pub struct Provider {
    behaviors: Mutex<VecDeque<Behavior>>,
    calls: AtomicUsize,
    requests: Mutex<Vec<Request>>,
}

impl Provider {
    /// A provider that follows `behaviors`, in order.
    #[must_use]
    pub fn new(behaviors: Vec<Behavior>) -> Self {
        Self {
            behaviors: Mutex::new(behaviors.into()),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// A provider that always does the same thing.
    #[must_use]
    pub fn always(behavior: Behavior) -> Self {
        Self::new(vec![behavior])
    }

    /// How many times it was called.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    /// The requests it received, in order.
    #[must_use]
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().expect("requests lock").clone()
    }

    /// The models the requests asked for, in order, skipping the ones that named none.
    #[must_use]
    pub fn models(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter_map(|request| request.model)
            .collect()
    }

    /// Answers one request, and records it.
    pub fn answer(&self, body: &[u8]) -> Answer {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let text = String::from_utf8_lossy(body).into_owned();
        let model = model_of(&text);
        self.requests.lock().expect("requests lock").push(Request {
            body: text,
            model: model.clone(),
        });

        match self.next() {
            Behavior::Ok {
                content,
                prompt_tokens,
                completion_tokens,
            } => Answer::Responded {
                status: 200,
                body: echo_model(
                    &usage_body(&content, prompt_tokens, completion_tokens),
                    model.as_deref(),
                ),
            },
            Behavior::Responds { status, body } => Answer::Responded {
                status,
                body: echo_model(&body, model.as_deref()),
            },
            Behavior::Stream(chunks) => Answer::Streamed {
                status: 200,
                chunks,
            },
            Behavior::NotSent => Answer::NotSent,
            Behavior::SentWithoutResponse => Answer::SentWithoutResponse,
        }
    }

    fn next(&self) -> Behavior {
        let mut list = self.behaviors.lock().expect("behaviors lock");
        if list.is_empty() {
            return Behavior::ok("ok");
        }
        if list.len() == 1 {
            return list[0].clone();
        }
        list.pop_front().unwrap_or_else(|| Behavior::ok("ok"))
    }
}

/// A chat-completion body with one message and no usage.
#[must_use]
pub fn message_body(content: &str) -> String {
    serde_json::json!({
        "choices": [{"message": {"content": content}}],
        "usage": {"prompt_tokens": 0, "completion_tokens": 0}
    })
    .to_string()
}

/// A chat-completion body with one message and the usage a gateway meters.
#[must_use]
pub fn usage_body(content: &str, prompt_tokens: u64, completion_tokens: u64) -> String {
    serde_json::json!({
        "choices": [{"message": {"content": content}}],
        "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": completion_tokens}
    })
    .to_string()
}

/// A chat-completion body that calls a tool, with the arguments as a JSON string, which is what
/// the provider format asks for.
#[must_use]
pub fn tool_call_body(id: &str, name: &str, arguments: &str) -> String {
    serde_json::json!({
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {"tool_calls": [{"id": id, "function": {"name": name, "arguments": arguments}}]}
        }],
        "usage": {"prompt_tokens": 1_000, "completion_tokens": 500}
    })
    .to_string()
}

/// The model a request body names, when it is JSON and names one.
#[must_use]
pub fn model_of(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("model")?
        .as_str()
        .map(str::to_owned)
}

/// Puts the requested model into a response body, the way a real provider does.
///
/// This is not cosmetic. Without it the model recorded downstream is `unknown`, a replay of that
/// record no longer matches the original run, and the difference has nothing to do with the code
/// under test. A body that is not JSON is returned untouched.
#[must_use]
pub fn echo_model(body: &str, model: Option<&str>) -> String {
    let Some(model) = model else {
        return body.to_owned();
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
        return body.to_owned();
    };
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "model".to_owned(),
            serde_json::Value::String(model.to_owned()),
        );
    }
    value.to_string()
}

#[cfg(feature = "http")]
pub mod http;
