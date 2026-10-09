//! The in-process transport: what the script answers, and what it remembers.

use test_provider::{
    echo_model, message_body, model_of, tool_call_body, usage_body, Answer, Behavior, Provider,
};

const REQUEST: &str = r#"{"model":"gpt-4o-mini","messages":[{"role":"user","content":"hello"}]}"#;

#[test]
fn a_script_is_followed_in_order_and_the_last_entry_repeats() {
    let provider = Provider::new(vec![
        Behavior::ok("first"),
        Behavior::Responds {
            status: 503,
            body: "{}".to_owned(),
        },
    ]);

    for (call, expected) in [(0, 200), (1, 503), (2, 503)] {
        let answer = provider.answer(REQUEST.as_bytes());
        let status = match answer {
            Answer::Responded { status, .. } => status,
            other => panic!("call {call}: expected a response, got {other:?}"),
        };
        assert_eq!(status, expected, "call {call}");
    }
    assert_eq!(provider.calls(), 3);
}

#[test]
fn an_ok_behavior_carries_the_usage_the_gateway_meters() {
    let provider = Provider::always(Behavior::usage("hi", 12, 7));
    let Answer::Responded { status, body } = provider.answer(REQUEST.as_bytes()) else {
        panic!("expected a response");
    };

    assert_eq!(status, 200);
    let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
    assert_eq!(value["choices"][0]["message"]["content"], "hi");
    assert_eq!(value["usage"]["prompt_tokens"], 12);
    assert_eq!(value["usage"]["completion_tokens"], 7);
}

#[test]
fn every_response_echoes_the_model_the_request_asked_for() {
    // the behavior that drifted between the copies: a real provider echoes the model, and without
    // the echo a recorded run says "unknown" and stops replaying. `Ok` builds its own body, so it
    // is the one that forgot the echo — which is what a lab over the wire noticed first
    for behavior in [
        Behavior::Responds {
            status: 200,
            body: r#"{"choices":[{"message":{"content":"hi"}}]}"#.to_owned(),
        },
        Behavior::usage("hi", 12, 7),
    ] {
        let provider = Provider::always(behavior);
        let Answer::Responded { body, .. } = provider.answer(REQUEST.as_bytes()) else {
            panic!("expected a response");
        };

        let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
        assert_eq!(value["model"], "gpt-4o-mini", "in {body}");
        assert_eq!(provider.models(), vec!["gpt-4o-mini".to_owned()]);
    }
}

#[test]
fn a_body_that_is_not_json_is_forwarded_untouched() {
    let provider = Provider::always(Behavior::Responds {
        status: 500,
        body: "not json at all".to_owned(),
    });
    let Answer::Responded { status, body } = provider.answer(REQUEST.as_bytes()) else {
        panic!("expected a response");
    };

    assert_eq!(status, 500);
    assert_eq!(body, "not json at all");
}

#[test]
fn the_two_failure_modes_are_reported_separately() {
    // not sent and sent-without-response are different promises: one may be retried for free,
    // the other may charge twice
    assert_eq!(
        Provider::always(Behavior::NotSent).answer(REQUEST.as_bytes()),
        Answer::NotSent
    );
    assert_eq!(
        Provider::always(Behavior::SentWithoutResponse).answer(REQUEST.as_bytes()),
        Answer::SentWithoutResponse
    );
}

#[test]
fn a_stream_is_answered_as_chunks_and_not_accumulated() {
    let provider = Provider::always(Behavior::Stream(vec![
        "data: one".to_owned(),
        "data: two".to_owned(),
    ]));
    let Answer::Streamed { status, chunks } = provider.answer(REQUEST.as_bytes()) else {
        panic!("expected a stream");
    };

    assert_eq!(status, 200);
    assert_eq!(chunks, vec!["data: one".to_owned(), "data: two".to_owned()]);
}

#[test]
fn a_provider_with_an_empty_script_still_answers() {
    // an empty script is a mistake, and a mistake should answer rather than hang
    let provider = Provider::new(Vec::new());
    assert!(matches!(
        provider.answer(REQUEST.as_bytes()),
        Answer::Responded { status: 200, .. }
    ));
}

#[test]
fn the_body_builders_escape_json() {
    // the bug the first copy of this code had: four escaped closing braces in a row and one too few
    let body = usage_body(r#"a "quoted" \ name"#, 1, 2);
    let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
    assert_eq!(
        value["choices"][0]["message"]["content"],
        r#"a "quoted" \ name"#
    );

    let call = tool_call_body("call_1", "search", r#"{"from":"NAP"}"#);
    let value: serde_json::Value = serde_json::from_str(&call).expect("call is JSON");
    assert_eq!(value["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(
        value["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
        r#"{"from":"NAP"}"#
    );
}

#[test]
fn the_small_helpers_do_one_thing_each() {
    assert_eq!(model_of(REQUEST), Some("gpt-4o-mini".to_owned()));
    assert_eq!(model_of("not json"), None);
    assert!(message_body("x").contains(r#""content":"x""#));
    assert_eq!(echo_model("not json", Some("m")), "not json");
    assert_eq!(echo_model("{}", None), "{}");
}
