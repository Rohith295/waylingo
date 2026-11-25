# Errors

A call either **throws** (the request or configuration is unusable, or the host gave
up) or **returns** a result whose `status` says how it went. Map provider and LLM
failures are not thrown: the result still says what was understood and what was found,
and the app needs that to tell the end user something useful.

## Thrown

In Rust, `WaylingoError` (`crates/waylingo/src/engine/mod.rs`) covers misuse:

| Variant | When |
| :--- | :--- |
| `InvalidRequest` | empty or over-long query, too many saved places, bad coordinates, settings out of range; on reroute, two choices for one stop or one place for two stops |
| `UnknownResponse` | a response with an id the session did not issue |
| `MissingResponses` | some requests of the last batch got no response |
| `Finished` | `step` after the session is done |
| `IncompatibleResult` | `resume` got something it cannot continue |
| `UnknownChoice` | a choice names something the previous result did not offer |

The reqwest hosts add `DriveError::Client` (the HTTP client could not be built) and
`DriveError::Timeout` (the async host's deadline passed).

The bindings raise one exception type with these variants
(`crates/waylingo-ffi/src/error.rs`), each with a `detail` string:

| Exception | When | What to do |
| :--- | :--- | :--- |
| `InvalidConfig` | an empty key without a proxy, a bad `base_url`, an empty model name, `timeout_ms` under 1,000, `max_map_calls` under 3, settings out of range | fix the configuration |
| `InvalidRequest` | as above | fix the input |
| `IncompatibleResult` | not a result from this instance: another version, provider or signing key; no plan | plan again |
| `UnknownChoice` | a choice not offered by the result | rebuild choices from the latest result |
| `Timeout` | the query took longer than `timeout_ms` | retry; raise `timeout_ms` if it happens often |
| `Internal` | a bug in Waylingo | report it |

The field is called `detail`, not `message`, because `message` clashes with the
built-in property of Kotlin and Java exceptions.

## Returned: status and error code

`status` is `complete` (every requested stop found; a trip may have none), `partial`
(the route was made, some stops were not found) or `failed` (no usable trip; see
`error.code` and `error.message`).

The codes are one fixed list (`ErrorCode` in `crates/waylingo/src/error_code.rs`). The
wire names are a public contract tested in `wire_names_are_stable`. Codes are only ever
added. A code from a newer version is read as `UNKNOWN`. Swift and Kotlin generate a
closed enum, so a `switch`/`when` without a default branch stops compiling when a code
is added. Include one.

| Code | Cause | Retryable | Needs user |
| :--- | :--- | :---: | :---: |
| `NOT_UNDERSTOOD` | not a trip request, or nothing was asked for | | yes |
| `NEEDS_DESTINATION` | no destination named and no active route passed | | yes |
| `NEEDS_CURRENT_LOCATION` | the trip starts here, but no `current_location` was passed | | yes |
| `INVALID_PLAN` | the model's plan stayed invalid after the repairs | | yes |
| `NO_GEOCODE` | a place in the request was not found | | yes |
| `NO_ROUTE` | the provider found no route between the places | | yes |
| `LLM_REFUSED` | the model declined | | yes |
| `LLM_AUTH` | LLM 401 or 403 | | |
| `LLM_QUOTA_EXCEEDED` | LLM 402, or a 429 that says the quota is used up | | |
| `LLM_RATE_LIMITED` | LLM 429 after the retries | yes | |
| `LLM_BAD_REQUEST` | other LLM 3xx/4xx: usually a wrong model name or base URL | | |
| `LLM_BAD_RESPONSE` | a 2xx in a format Waylingo cannot read | | |
| `LLM_UNAVAILABLE` | LLM 5xx or timeouts after the retries | yes | |
| `LLM_UNREACHABLE` | no connection to the LLM | yes | |
| `PROVIDER_AUTH` | map 401 or 403 (TomTom also uses 403 for some over-limit cases) | | |
| `PROVIDER_QUOTA_EXCEEDED` | map 402 | | |
| `PROVIDER_RATE_LIMITED` | map 429 after the retries | yes | |
| `PROVIDER_BAD_REQUEST` | other map 4xx | | |
| `PROVIDER_BAD_RESPONSE` | a map reply Waylingo cannot read | | |
| `PROVIDER_UNAVAILABLE` | map 5xx after the retries | yes | |
| `PROVIDER_UNREACHABLE` | no connection to the map provider | yes | |
| `INTERNAL` | a bug in Waylingo or in the host | | |
| `UNKNOWN` | a code from a newer version | | |

`is_retryable(code)` and `needs_user(code)` group the codes; a test checks that no code
is in both groups (`retry_and_ask_user_do_not_overlap`). The rest (auth, quota, bad
request or response) need the developer: retrying will not help and the end user cannot
fix them.

Why the "needs user" codes exist: Waylingo never asks the end user anything itself, and
it does not let the model guess what is missing. `NEEDS_DESTINATION` means the app should
ask "where to?"; `NEEDS_CURRENT_LOCATION` means the app should pass the location, or ask
where to start. See [Missing context is not repaired](llm-step.md#missing-context-is-not-repaired).

Which failures fail the whole trip:

| Failure | Effect |
| :--- | :--- |
| the LLM step | `failed`, no map calls made |
| geocoding a place | `failed` with `NO_GEOCODE`, or the provider error's code |
| the base route | `failed` with the provider error's code |
| TomTom's category list | warning; searches use text only |
| one stop's search | that stop is unresolved with `PROVIDER_ERROR` |
| a HERE matrix | warning; that stop is ranked by distance from the anchor |
| the final route | warning; the base route and anchor ETAs are used (on a reroute with no base route to fall back on, `failed`) |

## Unresolved stops

A stop without a place does not fail the trip. It appears as an `unresolved_stop`
feature (in the bindings, in `unresolved`) with a `result` (`reason` in the bindings):

| Reason | Meaning |
| :--- | :--- |
| `NO_CANDIDATES` | nothing matched within the detour limit, every match serves another stop, or (with `strict_radius`) nothing was within the radius; `detail` gives the counts |
| `SKIPPED_BY_USER` | the end user left it out; `options` keeps the places it offered |
| `SKIPPED_BUDGET` | the map call budget ran out before it was searched |
| `UNSUPPORTED` | the anchor cannot be placed on this route (for example, the route is shorter than the interval, or an offset lands past the end) |
| `PROVIDER_ERROR` | the provider failed for this search; `error_code` says how, with the codes above |
| `NOT_SEARCHED` | the search was set up but the result was built before its answer was processed. The engine processes every search answer before building a result, so a normal run does not produce it; it is a fallback (`output.rs`), and it is copied unchanged when a previous result carried it |

For `PROVIDER_ERROR`, `is_retryable(error_code)` tells the app whether trying again
later may help (for example `PROVIDER_RATE_LIMITED`). A stop-only reroute does not
search unresolved stops again; see [Reroute](reroute.md#what-is-fetched-again).
