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
print("offline checks passed")
