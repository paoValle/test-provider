# test-provider

> A provider that does exactly what it is told, for tests that would otherwise invent their own.

Every project that tests against an LLM writes this: a fake provider with a scripted answer, because
a test that hits a real API fails for reasons that have nothing to do with the code under test.

The fake then grows, and the copies drift. That is not hypothetical — it is why this crate exists.
An HTTP fake in one project had to learn to **echo the model** the request asked for, because a real
provider does, and without the echo the agent's recorded trace says `model: "unknown"` and its own
replay stops matching the original run. The other copies of the fake never learned it — and
`Behavior::Ok` in this crate did not either, until the fix in the changelog. A behaviour added to
one copy is invisible in the others, and the day it matters is the day a test is green for the wrong
reason.

```rust
use test_provider::{Behavior, Provider};

let provider = Provider::new(vec![
    Behavior::usage("the cheapest is Wizz at 41 euros", 1_000, 500),  // first call
    Behavior::unavailable(),                                          // second call
]);

match provider.answer(request_body) {
    Answer::Responded { status, body } => { /* forward it */ }
    Answer::NotSent => { /* the request never left: retry for free */ }
    Answer::SentWithoutResponse => { /* the request did leave: retrying may charge twice */ }
    Answer::Streamed { status, chunks } => { /* forward the chunks */ }
}
assert_eq!(provider.calls(), 1);
assert_eq!(provider.models(), vec!["gpt-4o-mini"]);
```

## What it does

- **Behaviours as data.** `Ok`, `Responds`, `NotSent`, `SentWithoutResponse`, `Stream` — one entry
  per call, and the last entry repeats, so "always unavailable" is one line.
- **The model is echoed** into the JSON response, the way a real provider echoes it — when the
  request named a model and the body is a JSON object; anything else is forwarded untouched. This is
  the behaviour that drifted between hand-written copies, and the one thing here that is not
  cosmetic.
- **It counts and it remembers.** `calls()`, `requests()`, `models()` — the assertions a test of a
  router, a budget or a failover policy actually needs.
- **Two transports.** `Provider::answer` in process, and `http::serve` (feature `http`) on a loopback
  port for the code that talks over a socket.
- **Bodies built by `serde_json`.** Hand-escaped JSON in the first copy of this code had four closing
  braces in a row and one too few.

## What it is not

- **Not a mock of a provider's behaviour.** It answers what it is told, and it does not model an
  API's semantics, rate limits or tokenizer.
- **Not the glue to your trait.** A test crate that owned `llmgateway`'s `Upstream` would have to be
  released in lockstep with `llmgateway`. Each consumer writes its own twenty-line adapter onto
  `Answer`, and that adapter is the only part that cannot be shared.
- **Not for the two failure modes over HTTP.** A provider that answers with a status has answered:
  `NotSent` and `SentWithoutResponse` have no HTTP shape, so over HTTP they are a `500` that says so.
  They are what the in-process transport is for.

## Development

```bash
make ci      # fmt-check, clippy with warnings as errors, the suite with all features
make test    # just the suite
```

The HTTP transport is a feature, and CI builds it with `--all-features`: a feature nobody compiles is
a feature that is broken without anyone knowing.
