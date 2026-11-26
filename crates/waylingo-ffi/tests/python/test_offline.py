"""Offline checks of the installed Python package: it loads, and bad input is
rejected with the documented exceptions. No network, no keys."""

import waylingo as w

cfg = w.WaylingoConfig(
    map_provider=w.MapProvider.TOMTOM,
    llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None),
)
assert w.version()

for args in [(cfg, "", "llm-key"), (cfg, "map-key")]:
    try:
        w.Waylingo(*args)
    except w.WaylingoError.InvalidConfig:
        pass
    else:
        raise AssertionError(f"{args[1:]!r} was accepted")

app = w.Waylingo(cfg, "map-key", "llm-key")
try:
    app.reroute_blocking("{}", w.Choices())
except w.WaylingoError.IncompatibleResult:
    pass
else:
    raise AssertionError("a non-Waylingo result was accepted")
# parse reports an unreachable LLM as a typed failure and makes no map call.
local = w.Waylingo(
    w.WaylingoConfig(
        map_provider=w.MapProvider.TOMTOM,
        llm=w.LlmBackend.OPEN_AI_COMPATIBLE(
            base_url="http://127.0.0.1:9/v1", model="m", fine_tuned=False),
    ),
    "map-key",
)
out = local.parse_blocking(w.TripInput(
    query="Drive to Utrecht", current_location=w.LatLng(lat=52.37, lng=4.89)))
assert out.status == w.ParseStatus.FAILED, out
assert out.error_code == w.ErrorCode.LLM_UNREACHABLE, out.error_code
assert out.plan_json is None
print("offline checks passed")
