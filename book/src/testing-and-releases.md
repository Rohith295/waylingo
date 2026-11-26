# Testing and releases

## Offline tests

`cargo test --workspace` needs no keys and no network. Because the engine does no I/O
(see [How a trip is planned](pipeline.md)), a test is a loop that answers each request
from a fixture.

| Area | What is tested | Where |
| :--- | :--- | :--- |
| Geometry | measured routes, anchors (including the slow-city case), windows, simplification, polylines | `crates/waylingo-geo/src/*.rs` |
| Plan | compilation, units, limits, tampered plans, plan hash | `crates/waylingo-plan/src/*.rs` |
| LLM step | request shapes per mode, repair, retries, error codes, the destination question, name grounding, no coordinates in requests | `crates/waylingo-parse/src/*.rs` |
| Adapters | request building and response parsing; no key in any URL | `crates/waylingo-providers/tests/tomtom.rs`, `here.rs` |
| Engine end to end | query in, GeoJSON out with a fake LLM and TomTom fixtures; byte-identical output for identical inputs; failures and their codes; settings | `crates/waylingo/tests/e2e.rs` |
| Reroute | one map call for a stop change, re-search for a new destination, signing, tampering, undo, skip and bring back | `crates/waylingo/tests/resume.rs` |
| Hosts | keys placed as the request says, redacted `Debug`, pacing across queries | `crates/waylingo-reqwest/src/*.rs` |
| Bindings | the library loads; bad configuration and bad results are rejected | `crates/waylingo-ffi/tests/python/test_offline.py`, `tests/kotlin-jvm` |

The TomTom fixtures are in `crates/waylingo-providers/tests/fixtures/tomtom/`; the HERE
tests use small synthetic JSON written inline. The fake LLM in
`crates/waylingo/tests/common/mod.rs` returns fixed plans, and some fixture place names
contain `WAYLINGO_CANARY_<n>`. `provider_data_never_reaches_the_llm` drives a full trip
and fails if any LLM request comes after a map request, contains a canary string or the
user's coordinates, or if any URL carries a key.

## Live contract tests

Offline fixtures cannot notice when a provider changes its response format, retires a
model or changes authentication. Live tests do, and they are all `#[ignore]`d so a
normal `cargo test` never calls a real API.

* **Map providers, nightly.** `.github/workflows/live.yml` runs at 04:17 UTC every day
  (and by hand). It runs the raw TomTom checks
  (`crates/waylingo-providers/tests/live_tomtom.rs`) and the whole engine against TomTom
  and HERE (`crates/waylingo-reqwest/tests/live_maps.rs`): geocode, route, search, detour
  ranking and reroute. The LLM step is answered with a fixed plan, so only map keys are
  needed and a map-side change is caught on its own. `WAYLINGO_LIVE_REQUIRE` turns a
  missing key into a failure, so a scheduled run cannot pass by skipping.
* **LLM backends, by hand.** `crates/waylingo-ffi/tests/live.rs` goes through the same
  public API the bindings use, for TomTom and HERE with Anthropic, and TomTom with
  OpenAI and Gemini, plus "not a trip", wrong keys and destination-only trips. It is not
  scheduled.
* **Scenarios, by hand.** `crates/waylingo-reqwest/examples/scenarios.rs` runs every
  supported intent against the real LLM and map provider and checks the results.

Live tests print statuses, codes and counts only: no provider place names, addresses or
coordinates, and never keys.

## CI

`.github/workflows/ci.yml`, on every push to `main` and every pull request, with no
secrets and no network tests:

* Rust on Linux, macOS and Windows: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`,
  and (not on Windows)
  `scripts/check_deps.sh`, which fails if `waylingo-parse` and `waylingo-providers`
  depend on each other in either direction;
* Python: build the wheel with maturin, install it, run the offline test;
* Kotlin: build the library, generate the Kotlin bindings, compile and test them on the
  JVM with Gradle.

The workspace lints forbid `unsafe` code in every crate except `waylingo-ffi`
(`Cargo.toml`), whose generated UniFFI scaffolding needs it; the hand-written code in
`waylingo-ffi` has none.

## Releases

`.github/workflows/release.yml` is started by hand with a version, which must match the
workspace version in `Cargo.toml`. With `publish` off, the default, it is a dry run:
everything is built and kept as workflow artifacts, and nothing leaves GitHub.

It builds:

* Python wheels (manylinux and musllinux for x86_64 and aarch64, macOS universal2,
  Windows x64) and an sdist;
* the Swift XCFramework (iOS device, iOS simulator, macOS) and the generated Swift
  source, and checks that `Package.swift` compiles against it;
* the Android native libraries (arm64-v8a, armeabi-v7a, x86_64), the Kotlin bindings,
  and the AAR.

With `publish` on, it also points `Package.swift` at the release, commits the Swift
source, tags the version, creates the GitHub Release with the XCFramework and the AAR,
and publishes to PyPI (trusted publishing, no stored token) and Maven Central. Each
publish job runs in a GitHub environment that can require a manual approval.

Version 0.1.0 was released this way. The crates are published to crates.io by hand
with `cargo publish`, not by the workflow.

## Pull requests

The project does not accept code contributions. `.github/workflows/pull-requests.yml`
closes pull requests from anyone but the owner with a short note. It uses
`pull_request_target` and never checks out or runs the pull request's code.
