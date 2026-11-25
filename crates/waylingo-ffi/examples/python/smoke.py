"""Live smoke test of the Python bindings: plan, reroute, async, errors.

Needs TOMTOM_API_KEY and LLM_API_KEY (Anthropic) in the environment or in .env at
the repository root. Prints only statuses, counts and ids, never place names.

    maturin develop --release   # in crates/waylingo-ffi
    python examples/python/smoke.py
"""

import asyncio
import os
import pathlib
import sys

import waylingo as w


def load_env() -> None:
    env = pathlib.Path(__file__).resolve().parents[4] / ".env"
    if env.exists():
        for line in env.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, v = line.split("=", 1)
                os.environ.setdefault(k.strip(), v.strip().strip('"').strip("'"))


def summary(out: w.TripOutput) -> dict:
    return {
        "status": out.status.name,
        "error_code": out.error_code,
        "stops": len(out.stops),
        "unresolved": len(out.unresolved),
        "llm_calls": out.llm_calls,
        "map_calls": out.map_calls,
        "has_attribution": bool(out.attribution),
    }


def main() -> int:
    load_env()
    map_key, llm_key = os.environ.get("TOMTOM_API_KEY"), os.environ.get("LLM_API_KEY")
    if not map_key or not llm_key:
        print("set TOMTOM_API_KEY and LLM_API_KEY")
        return 2

    cfg = w.WaylingoConfig(
        map_provider=w.MapProvider.TOMTOM,
        llm=w.LlmBackend.ANTHROPIC(model=None, base_url=None),
        settings=w.SearchSettings(top_n=3),
    )
    app = w.Waylingo(cfg, map_key, llm_key)
    trip = w.TripInput(
        query="Drive to Utrecht, coffee halfway",
        current_location=w.LatLng(lat=52.3702, lng=4.8952),
        locale="en-GB",
    )
    failures = 0

    out = app.plan_blocking(trip)
    s = summary(out)
    print("plan (blocking):", s)
    failures += out.status == w.TripStatus.FAILED

    stop = next((s for s in out.stops if s.alternatives), None)
    if stop:
        alt = stop.alternatives[0].provider_place_id
        choices = w.Choices(
            stops=[
                w.StopChoice(
                    stop_request_id=stop.stop_request_id,
                    occurrence=stop.occurrence,
                    pick=w.StopPick.PLACE(provider_place_id=alt),
                )
            ]
        )
        again = app.reroute_blocking(out.geojson, choices)
        r = summary(again)
        chosen = next(s for s in again.stops if s.stop_request_id == stop.stop_request_id)
        print("reroute:", r, "user_choice:", chosen.user_choice)
        ok = (
            r["llm_calls"] == 0
            and r["map_calls"] == 1
            and chosen.pick.provider_place_id == alt
            and chosen.user_choice == w.UserChoice.CHOSEN
        )
        print("reroute used no LLM, one map call, and the chosen place:", ok)
        failures += not ok
    else:
        print("no alternatives to choose from; reroute not exercised")

    async def plan_async() -> w.TripOutput:
        return await app.plan(trip)

    a = asyncio.run(plan_async())
    print("plan (async):", summary(a))
    failures += a.status == w.TripStatus.FAILED

    try:
        w.Waylingo(cfg, map_key)  # no LLM key and no proxy
        print("missing LLM key accepted: WRONG")
        failures += 1
    except w.WaylingoError.InvalidConfig as e:
        print("missing LLM key rejected:", type(e).__name__)
    try:
        w.Waylingo(cfg, "", llm_key)
        print("empty map key accepted: WRONG")
        failures += 1
    except w.WaylingoError.InvalidConfig as e:
        print("empty map key rejected:", type(e).__name__)
    try:
        app.reroute_blocking("{}", w.Choices())
        print("garbage result accepted: WRONG")
        failures += 1
    except w.WaylingoError.IncompatibleResult as e:
        print("garbage result rejected:", type(e).__name__)

    print("FAILURES:", failures)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
