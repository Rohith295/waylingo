# Glossary

**Alternative.** Another place offered for a stop, ranked after the pick. At most
`top_n − 1` per stop, and always on the same side of the radius as the pick.

**Anchor.** Where on the route a stop is wanted: an offset from the origin, the
destination or a waypoint ("15 minutes before we arrive"), every N ("every 200 km"),
or a fraction ("halfway"). Resolved to one or more anchor points on the base route.

**Candidate.** Two meanings. A *geocoding candidate* is one match for a place name the
user said (up to 5 per place, for "did you mean ...?"). A *search candidate* is one
place returned by an along-route search, before the pick is chosen.

**Intent** (`SurfaceIntent`). The JSON the LLM writes: places and units as the user
said them. See [The plan](plan.md).

**Mark.** One anchor point of an "every N" anchor: at N, 2N, 3N, ... from the start.

**Occurrence.** The number of a mark: 0, 1, 2, ... A stop is identified by its stop
request id plus its occurrence. Single anchors have occurrence 0.

**Option.** A place offered by a stop the end user skipped, kept so the stop can be
brought back.

**Pick.** The place chosen for a stop; the final route goes through it.

**Pin.** An end user's earlier stop choice carried into a re-search after the origin,
a waypoint or the destination changed. It is kept if the same place is found again.

**Plan** (`Plan`). The compiled, validated form of the intent: metres, seconds,
coordinates or geocoding text, checked references. See [The plan](plan.md).

**Radius** (`anchor_radius_m`). Places within this straight-line distance of an anchor
point are preferred. Default 5 km.

**Span** (`search_span_m`). How much route is searched on each side of an anchor point.
Default 3 × the radius.

**Window.** The stretch of route searched for one anchor point: from anchor − span to
anchor + span, clamped to the route.
