# Reroute and the interaction loop

A planned trip is a proposal. The end user may prefer another charger, not want the
coffee stop, or have meant a different "Springfield". Waylingo handles this without a
second LLM call and without keeping any state between calls.

## The loop in an app

1. The end user types or says a trip.
2. `plan` returns a result: the route, each stop's pick and alternatives, unresolved
   stops, and each named place with its geocoding candidates.
3. The app shows the options.
4. The end user picks another place for a stop, skips a stop, or picks another
   candidate for the origin, a waypoint or the destination.
5. `reroute(previous_result, choices)` returns a new result. Go to 3.

To undo, keep the earlier result and reroute from it instead.

## Stateless resume

`Engine::resume(previous, choices, request_id)` (`crates/waylingo/src/engine/resume.rs`)
rebuilds a session from the previous result document alone. The plan, the search
settings, every location with its candidates, every stop with its anchor, window, pick
and alternatives are all in the document. The bindings call this `reroute`.

Why stateless: a server does not need a session store or sticky routing, a phone app
can resume after being killed, and any earlier result can be resumed to undo. The cost
is that the previous result is now input from the client, so it is checked like input.

## Choices

`Choices` (`crates/waylingo/src/choices.rs`) has two lists:

* `stops`: for a `(stop_request_id, occurrence)`, either `Place { provider_place_id }`
  (the pick, one of the alternatives, or for a skipped stop one of its `options`) or
  `Skip`;
* `locations`: for the origin, a waypoint (by index) or the destination, a
  `candidate_id` from that place's `candidates`.

Choices recorded in the previous result carry over: a pick marked
`user_choice: "chosen"` and a stop with `result: "SKIPPED_BY_USER"`. A new choice for
the same stop replaces the old one.

## What is fetched again

| Change | LLM | Geocode | Base route | Searches | Matrices (HERE) | Final route |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| Stop choices only (pick, skip, bring back) | no | no | no | no | no | 1 call |
| Another origin, waypoint or destination candidate | no | no | yes | yes | yes | yes |
| Previous result had no route | no | no | yes | yes | yes | yes |
| No choices, previous result had a route | no | no | no | no | no | 1 call |

When the base route is redone, TomTom's category list is also fetched again (one call,
only when the plan has stops), because a resumed session starts with a fresh adapter
(`Engine::resume` in `resume.rs`). A previous result that never got as far as a route
is searched again from the base route in the same way.

**Stop choices only.** The stops are rebuilt from the document, the choices are
applied, and one route call goes through the chosen places
(`a_stop_choice_reroutes_with_one_map_call_and_no_llm` in
`crates/waylingo/tests/resume.rs`). A newly picked place becomes the pick; the old pick
becomes the first alternative, so the end user can switch back. A skipped stop keeps
the places it offered as `options`, so it can be brought back later.

**A different location.** Everything from the base route on is redone, because stops
anchored to the destination ("15 minutes before we arrive") move with it. The new
position comes from the chosen geocoding candidate, so no new geocoding call is made.
Earlier stop choices become *pins*: if the same place is found again it is kept (and
kept in the matrix shortlist), otherwise the new best match is used and marked
`user_choice: "dropped"`, with a warning. A stop skipped earlier stays skipped and is
not searched; it then has no options, so to bring it back, resume the earlier result.

**What is not refreshed.** On a stop-only reroute, stops that were unresolved in the
previous result (`NO_CANDIDATES`, `PROVIDER_ERROR`, `SKIPPED_BUDGET`, ...) are copied as
they are; they are not searched again. Travel times come from the new route call, but
candidates, detours and alternatives are the ones found by the original search.

`usage` in the new result counts only the calls made by the reroute.

## Checks on the previous result

`resume` rejects, with `IncompatibleResult`:

* text that is not a Waylingo result, or a different major result schema version or a
  different plan schema version;
* a result containing an error code this version does not know (it was made by a newer
  Waylingo);
* when the engine has a signing key: a missing or wrong signature;
* a result without a plan (the request was never understood);
* a plan that fails `Plan::validate`, or search settings out of range;
* a result from another map provider;
* positions off the globe, invalid distances, a waypoint without `along_m`;
* the same stop (`stop_request_id` and `occurrence`) appearing twice;
* a stop whose `stop_request_id` is not in the plan;
* a geocoded place with no chosen candidate.

And it rejects bad choices: `UnknownChoice` for a stop, place or candidate the result
did not offer, and `InvalidRequest` when one stop gets two choices or one place is
chosen for two stops. Positions of geocoded places always come from the chosen
candidate, not from the feature geometry, so editing a location's geometry has no
effect (`a_tampered_destination_geometry_is_ignored`).

## Signing

Without a signing key, `resume` trusts the document's contents within the checks above.
For a phone app resuming its own results, that is enough. A server that takes results
back from clients should set a signing key (`EngineConfig::signing_key`, or
`signing_key` in the bindings' constructor; 32 random bytes are recommended). Then:

* every result carries `signature`: HMAC-SHA256, hex, over the document serialized with
  `signature: null` (`crates/waylingo/src/signing.rs`);
* `resume` accepts only documents whose signature verifies (constant-time comparison).

With a key, a client cannot inject a place Waylingo did not find, edit the plan to make
the server do more work, or move a stop.

**The route line is not signed.** Before computing the HMAC, the route feature's
geometry is replaced by an empty line. The reason is size: the route line is by far the
largest part of a result, and `resume` never reads it (positions come from candidates,
stops and anchors). So a client may empty the route's `coordinates`, or set its
`geometry` to null, before sending a result back, and the signature still verifies. The
route feature itself must stay.

Signing works because serialization is deterministic: field order is fixed by the
types, positions are rounded to 6 decimals, distances and times to whole metres and
seconds, scores to 3 decimals (`crates/waylingo/src/output.rs`). A test checks that
signatures survive values that are awkward to round-trip
(`signatures_survive_awkward_numbers`).

The format of the document itself is described in [The result document](result-document.md).
