# Design decisions and limitations

## Key decisions

Each decision is explained in the linked section.

| Decision | Reason |
| :--- | :--- |
| [The LLM only turns text into a fixed JSON plan](llm-step.md) | LLMs are unreliable at route arithmetic and detour ranking, and this keeps provider data out of the LLM |
| [The LLM is required; there is no public "run this plan" entry point](introduction.md#what-it-is-not) | Waylingo is a natural-language engine, not a routing SDK. The cost: an agent that already runs an LLM pays for a second LLM call |
| [The engine does no I/O](pipeline.md#the-engine-does-no-io) | keys stay with the host; bindings need no async callbacks; apps keep their own network stack; tests replay fixtures without HTTP mocking |
| [Two plan tiers: `SurfaceIntent` from the LLM, `Plan` from a compiler](plan.md#why-two-tiers) | the model copies units and names as written; Rust converts and checks, and can tell the model exactly what to fix |
| [Missing context is never repaired](llm-step.md#missing-context-is-not-repaired) | a model asked for a missing value can only invent it; the app asks the end user instead |
| [Names are grounded back to the request's words](llm-step.md#grounding-names-in-the-query) | small models garble unfamiliar names; the request is the only valid source for them |
| [One map provider per app](providers.md) | detours subtract travel times, which only makes sense within one provider's traffic model |
| [Anchors use the provider's distance and time checkpoints](routing-and-stops.md#the-measured-route) | highway and city speeds differ; an even spread puts time anchors in the wrong place |
| ["Every N" is counted from the start; anchors stay on the base route](routing-and-stops.md#anchors) | all searches are independent and go out in one parallel batch; one final route gives the real ETAs |
| [Prefer places within a radius, rank by detour time, flag a fallback beyond it](routing-and-stops.md#choosing-one-place-per-stop) | a place close in a straight line can cost a long detour; an answer with a clear flag beats "nothing found" |
| [HERE detours from a matrix over a shortlist](routing-and-stops.md#detour-time) | HERE search returns distance off the route, not time; the shortlist keeps it to one small call per stop |
| [Stateless reroute](reroute.md#stateless-resume) | no session store; resume after an app restart; undo by resuming an older result |
| [Optional signing that leaves out the route line](reroute.md#signing) | servers can trust results sent back by clients; clients can drop the largest part to save bandwidth |
| [Hard limits on plans and a map call budget](plan.md#validation-limits) | a misread request or a tampered result cannot make a server send thousands of requests |
| [A fixed, append-only list of error codes](errors.md) | apps can handle every case and know whether to retry or ask the end user |
| [Map pacing in the host, across queries](routing-and-stops.md#call-budget-retries-and-pacing) | only the host knows how many queries share the provider's rate limit |

## Found against the live APIs

Some choices came from how real endpoints behaved, not from their documentation:

* **TomTom categories:** a category stop without a brand searches for `*` plus
  `categorySet`, because generic text together with `categorySet` returned nothing
  (`search_request` in `crates/waylingo-providers/src/tomtom.rs`).
* **No TomTom `brandSet`:** it only matches chains in TomTom's brand registry, so a
  local name such as "Cafe de Jaren" would find nothing; the name as search text finds
  both chains (even misspelt) and single places (`tomtom.rs`).
* **`tool_choice: "auto"`:** some endpoints reject a forced tool call, so the tool is
  offered, not forced, and a text answer is read as well
  (`crates/waylingo-parse/src/openai.rs`).

## Known limitations

These are true of the code as it is now.

**Reroute**

* A stop-only reroute does not search again for stops that were unresolved in the
  previous result (`NO_CANDIDATES`, `PROVIDER_ERROR`, `SKIPPED_BUDGET`). They are copied
  as they were. Only a change of origin, waypoint or destination re-runs the searches.
* A stop skipped before a destination change has no options afterwards; to bring it
  back, resume the earlier result.
* Without a signing key, `reroute` trusts stop positions in the document as given.
  Servers that take results back from clients should set a key.

**Providers**

* TomTom answers 403 for some over-limit cases, so a used-up TomTom quota may surface as
  `PROVIDER_AUTH` rather than `PROVIDER_QUOTA_EXCEEDED`.
* HERE: no connector filter; the electric-vehicle flag is not sent on route requests;
  only the 8 shortlisted candidates (plus a chosen one) survive the detour step.
* There is no Google adapter, although `ProviderKind` has a `Google` value.

**Routing and stops**

* No EV range model. "Every 200 km" is spacing from the start of the trip, and the real
  gap between two chosen chargers can exceed 200 km (each pick may sit up to the radius
  from its mark, or further when the beyond-radius fallback is used).
* Anchors are not shifted by earlier stops' detours or time spent at a stop.
* "Halfway" and other fractions are by distance, not by time.
* `heading_deg` is accepted but not used. An active route is passed only as its
  destination; the route is recomputed from the current location.
* The timezone is carried in the plan but not sent to the providers.
* The call budget is checked before searches and matrices only; geocoding, routes and
  retries are counted but never refused.

**Pacing under load**

* Pacing is per `AsyncHost` (per `Waylingo` instance in the bindings). Several processes
  do not coordinate, so together they can exceed the provider's rate limit.
* Under heavy load, queries wait at the shared map gate, and that waiting counts against
  each query's deadline: queries then fail with `Timeout` instead of being queued
  longer. The blocking host waits a fixed 220 ms before each map request whatever the
  load.

**LLM step and parser model**

* The fine-tuned parser is English-only, trained on places in Europe, the US and India,
  and gets 5 of 10 hand-written addresses right. Trips with a destination and no stops
  are being added to its training data.
* Running the parser on the device is planned, not built.
* Gemini has not been tested live.

**Packaging**

* No package is published. Kotlin is tested on the JVM in CI but not on Android; iOS and
  Android device builds are not verified end to end; the release workflow has not had a
  first run.

## The original design document

`docs/designs/waylingo.md` is the original design, written before the code; this book
describes the current code.
