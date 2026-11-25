# The LLM step

The LLM has one job: turn the request text into a small JSON object, the
`SurfaceIntent` (see [The plan](plan.md)). It does not answer the user, does not choose
places and never sees map data. All of this lives in `crates/waylingo-parse/`.

## What the LLM can and cannot see

Every LLM request is built from three sources only:

1. a fixed system prompt (`crates/waylingo-parse/src/prompt.rs`);
2. the user message, built by `ParseInput::user_message`
   (`crates/waylingo-parse/src/input.rs`);
3. during repairs, the model's own earlier replies and a list of validation errors
   about them.

The user message looks like this:

```text
Locale: en-GB
Saved places: saved:home, saved:work
Request: drive home, coffee halfway
```

`ParseInput` is built from the query and the `TripContext`, but it copies only the
query, the locale and the saved-place *names*. It has no field for coordinates, the
active destination, the vehicle or anything from a provider, and no way to add one.
The `Conversation` type (`crates/waylingo-parse/src/conversation.rs`) is the only way
to add messages, and each of its methods accepts one of the allowed sources above.

`ParseInput::new` also rejects input before any call is made: an empty query, a query
over 2,000 characters, more than 50 saved places, a saved-place name that is empty or
over 60 characters, a locale over 35 characters, or coordinates out of range. These
become `InvalidRequest` errors.

## Prompt styles

`LlmConfig.prompt` selects one of two system prompts (`PromptStyle` in
`crates/waylingo-parse/src/config.rs`):

* **Full** (the default) is for general models. It states the rules (copy places as
  written, never write coordinates, omit the origin unless named, keep the user's units,
  how to map phrases to anchors, brand versus category, the `not_understood` answer;
  "fast charger" with no number means 150 kW and "rapid" means 50 kW) and gives four
  worked examples (`prompt.rs`). In `prompt_json` mode the JSON Schema is appended.
* **Compact** is one line, `"Convert the trip request to Waylingo JSON."`
  (`COMPACT_SYSTEM_PROMPT`). It is for the [fine-tuned parser](fine-tuning.md), which was
  trained with exactly this string and learned the rules from data. No schema is
  appended, even in `prompt_json` mode.

## Output modes

`LlmConfig.mode` selects how structured output is requested
(`crates/waylingo-parse/src/openai.rs`). All three use the OpenAI chat-completions
format at `{base_url}/chat/completions`.

| Mode | Request | Reply read from |
| :--- | :--- | :--- |
| `json_schema` | `response_format: {type: "json_schema", json_schema: {name: "surface_intent", strict: false, schema}}` | `message.content` |
| `tool_call` | one function, `submit_trip_intent`, whose parameters are the schema; `tool_choice: "auto"` | the tool call's `arguments`, or `content` if the model answered in text |
| `prompt_json` | nothing extra; the schema is in the full system prompt | `message.content` |

Details that came from real endpoints:

* The schema is generated from the Rust type with `schemars`, with sub-schemas inlined
  (no `$ref`) and the `$schema` key removed, because some endpoints do not resolve
  references in tool or response schemas (`crates/waylingo-plan/src/schema.rs`).
* `tool_choice` is `"auto"`, not a forced function, because some endpoints (the code
  comment names Anthropic's compatibility layer with some models) reject forced tool
  choice.
* `temperature` is left out unless configured, because some models reject it.
* In every mode, code fences and prose around the JSON are dropped by keeping the
  outermost `{ ... }`.
* An OpenAI-style `refusal` field ends the parse with `LLM_REFUSED`.

The bindings pick the mode per backend (`crates/waylingo-ffi/src/config.rs`): Anthropic
uses `tool_call`, OpenAI and Gemini use `json_schema`, and any other OpenAI-compatible
server uses `prompt_json` (with the compact prompt when `fine_tuned` is set).

## Validation, repair and retry

Each reply goes through the same steps (`Parser::on_response` in
`crates/waylingo-parse/src/parser.rs`):

```text
reply ──▶ HTTP error? ──yes──▶ retry (429, 5xx, transport) or fail with a code
            │no
            ▼
        JSON for SurfaceIntent? ──no──▶ repair
            │yes
            ▼
        "error": "not_understood"? ──yes──▶ NOT_UNDERSTOOD (no repair)
            │no
            ▼
        ground names in the query text
            ▼
        compile + validate ──ok──▶ Plan (parse done)
            │errors
            ▼
        missing context (needs)? ──yes──▶ NEEDS_* (destination: one neutral question first)
            │no
            ▼
        repairs left? ──yes──▶ repair     ──no──▶ INVALID_PLAN
```

**Repair.** The model's reply is added to the conversation as an assistant message,
followed by a user message listing every problem as a JSON Pointer into its own JSON,
a reason, and the allowed values where the field has a fixed set:

```text
Your JSON has these problems. Reply with the full corrected JSON object only.
- /stops/0/what/category: unknown category "cafe"; use text for other places (allowed: ev_charging, fuel, ...)
```

At most `max_repairs` (default 2) repairs are made, so a parse that never becomes
valid costs three LLM calls and ends with `INVALID_PLAN` and the last errors in the
message (`gives_up_after_max_repairs`). The `SurfaceIntent` type rejects unknown
fields, and its enum-like fields are plain strings so the compiler can answer "allowed
values: ..." instead of a bare JSON parse error.

**Retry.** A 429, a 5xx or a transport failure (other than cancellation) is retried up
to `max_retries` (default 2) times for the whole parse, with the same body. The wait
is the `Retry-After` header when present, otherwise 1 s, then 2 s, capped at 30 s,
passed to the host as `not_before_ms`. A cancelled request is not retried and ends as
`LLM_UNAVAILABLE`. Other failures are not retried:

| Response | Code |
| :--- | :--- |
| 401, 403 | `LLM_AUTH` |
| 402, or a 429 whose body says `insufficient_quota` or `quota exceeded` | `LLM_QUOTA_EXCEEDED` |
| 429 after the retries | `LLM_RATE_LIMITED` |
| other 3xx or 4xx (a redirect usually means a wrong base URL; a 4xx a wrong model name) | `LLM_BAD_REQUEST` |
| 5xx or timeout after the retries | `LLM_UNAVAILABLE` |
| connection failure after the retries | `LLM_UNREACHABLE` |
| 2xx that is not a chat-completions body | `LLM_BAD_RESPONSE` |

Error messages include the endpoint's own error text, cut to 300 characters, with any
word that starts with `sk-` or is longer than 40 characters replaced by `<redacted>`.

## Grounding names in the query

Small models sometimes garble names they never saw in training ("Bordeaux" becomes
"Bordean"). Every place, brand and free-text string in the plan should come from the
request, so after parsing, `ground_in_query` (`crates/waylingo-parse/src/ground.rs`)
checks each one:

1. If the string appears verbatim in the query (case-sensitive), keep it (trimmed).
2. Otherwise, compare it with every run of consecutive query words whose length is
   within one word of the string's word count. Skip runs whose first letter differs when
   the string starts with a letter (names rarely get their first letter wrong; house
   numbers do).
3. Take the run with the smallest Levenshtein distance relative to the longer length;
   on a tie, the shorter run.
4. Replace the string only if that relative distance is at most 0.5. Otherwise leave
   it alone: the geocoder may still resolve it (the request said "SF" and the model
   wrote "San Francisco": no run of words is close, so "San Francisco" is kept).

`saved:<name>` references and `current_location` are never touched, and neither are
categories. The tests use real failures from evaluation runs, for example "Zagad" to
"Zagreb", and "Terminal 5" to "Terminal 3" when the request said 3.

## Missing context is not repaired

Some compile errors are not mistakes the model can fix: the information is missing.
These errors carry a `needs` value (`Need` in `crates/waylingo-plan/src/compile.rs`)
and are never sent back as a repair, because the model could only invent the value:

| Need | When | Result |
| :--- | :--- | :--- |
| `Request` | nothing was asked for: no destination, waypoint or stop | `NOT_UNDERSTOOD` |
| `CurrentLocation` | the trip starts at the current location (no origin named) and the host passed none | `NEEDS_CURRENT_LOCATION` |
| `Destination` | no destination was named and the host passed no active route | `NEEDS_DESTINATION`, after one question |

**The one neutral question.** Models sometimes drop a destination the user did name.
So when a destination is missing and a repair slot is left, the parser asks once, even
if the reply has other errors (`Parser::on_response` in `parser.rs`). The question uses
the repair slot and a fixed text that does not suggest any answer. (Missing needs are
checked in the order the compiler reports them; the origin comes first, so a missing
current location is reported before the destination is asked about.)

```text
- /destination: missing. If the request names where to go, add it exactly as written there; otherwise leave it out
```

After this question a destination is accepted only if it appears in the request text
(case-insensitive, with a `saved:` prefix removed). An invented one, or none, ends the
parse with `NEEDS_DESTINATION` (`an_invented_destination_is_refused`,
`missing_destination_is_asked_once_then_reported` in `parser.rs`). The app should then
ask the end user "where to?".

**Not a trip.** For text that is not a trip request, the prompts and training data ask
for exactly `{"error":"not_understood","stops":[]}`. Any reply with `error` set ends the
parse with `NOT_UNDERSTOOD` and no repair.

## After the parse

When the plan compiles, the parser is dropped and the session moves to map calls. From
then on, no LLM request can be issued: the parser no longer exists, and the phase only
moves forward. A trip made with a hosted model therefore costs one LLM call in the
common case, and at most `1 + max_repairs + max_retries` calls.
