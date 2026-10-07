# Changelog

Format [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
versioning [SemVer](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-10-07

First version: the behaviours and the two transports, extracted because three hand-written copies of
the same fake had already drifted.

### Added
- `Behavior` as data (`Ok`, `Responds`, `NotSent`, `SentWithoutResponse`, `Stream`), one entry per
  call, the last one repeating.
- `Provider`: a script, a call counter, and the requests it received (`calls`, `requests`, `models`).
- `Answer`: what the provider decides to answer, for a consumer to map onto its own transport types.
- The model echo into every JSON response, which is the behaviour that had drifted, plus the body
  builders (`message_body`, `usage_body`, `tool_call_body`) built with `serde_json`.
- `http::serve` behind the `http` feature: the same script over a loopback socket, SSE chunks
  included, for the code that talks with an HTTP client.
