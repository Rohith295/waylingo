"""Waylingo demo: plan a trip from one sentence in your browser, with your own keys.

    pip install waylingo
    python demo/server.py

Then open http://127.0.0.1:8770. Keys are typed into the page and sent only to this
local server, which keeps them in memory for the calls you start. Nothing is written
to disk or logged. The page loads nothing from other sites: its fonts and map
library are in demo/static, and map tiles come through this server from your map
provider (or OpenStreetMap when your key has no map display access).
"""

import argparse
import hashlib
import json
import mimetypes
import secrets
import threading
import urllib.error
import urllib.parse
import urllib.request
from collections import OrderedDict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import waylingo as w

HERE = Path(__file__).parent
PAGE = HERE / "index.html"
STATIC = (HERE / "static").resolve()
LOCAL_HOSTS = {"127.0.0.1", "localhost", "::1"}
USER_AGENT = "waylingo-demo/1 (+https://github.com/Rohith295/waylingo)"

mimetypes.add_type("font/woff2", ".woff2")


class BadInput(Exception):
    """A missing or invalid setting, shown on the page."""


def is_local(url: str) -> bool:
    try:
        return urllib.parse.urlsplit(url).hostname in LOCAL_HOSTS
    except ValueError:
        return False


def name(value):
    return None if value is None else str(value).split(".")[-1]


def request(url: str) -> urllib.request.Request:
    return urllib.request.Request(url, headers={"User-Agent": USER_AGENT})


# ---------------------------------------------------------------- connections

def llm_backend(llm: dict):
    kind = llm.get("kind")
    key = (llm.get("key") or "").strip() or None
    model = (llm.get("model") or "").strip() or None
    if kind == "builtin":
        url = (llm.get("url") or "").strip().rstrip("/")
        if not is_local(url):
            raise BadInput("The built-in model must run on this computer (localhost).")
        backend = w.LlmBackend.OPEN_AI_COMPATIBLE(base_url=url, model="default", fine_tuned=True)
        return backend, None
    if not key:
        raise BadInput("Paste your LLM key, or choose the built-in model.")
    if kind == "anthropic":
        return w.LlmBackend.ANTHROPIC(model=model, base_url=None), key
    if kind in ("openai", "gemini"):
        if not model:
            raise BadInput("Enter the model name to use.")
        if kind == "openai":
            return w.LlmBackend.OPEN_AI(model=model, base_url=None), key
        return w.LlmBackend.GEMINI(model=model, base_url=None), key
    raise BadInput("Choose a language model.")


class Connection:
    """One set of keys and settings: a Waylingo instance and a tile source."""

    def __init__(self, settings: dict):
        mp = settings.get("map") or {}
        self.provider = mp.get("provider")
        self.map_key = (mp.get("key") or "").strip()
        if self.provider not in ("tomtom", "here"):
            raise BadInput("Choose TomTom or HERE.")
        if not self.map_key:
            label = "TomTom" if self.provider == "tomtom" else "HERE"
            raise BadInput(f"Paste your {label} key.")
        backend, llm_key = llm_backend(settings.get("llm") or {})
        map_provider = w.MapProvider.TOMTOM if self.provider == "tomtom" else w.MapProvider.HERE
        self.app = w.Waylingo(
            w.WaylingoConfig(map_provider=map_provider, llm=backend),
            map_key=self.map_key,
            llm_key=llm_key,
        )
        self.tiles = self.provider
        if not self._tiles_work():
            self.tiles = "osm"

    def tile_url(self, style: str, z: int, x: int, y: int) -> str:
        dark = style == "dark"
        if self.tiles == "here":
            q = urllib.parse.urlencode({"style": "explore.night" if dark else "explore.day",
                                        "size": 256, "apiKey": self.map_key})
            return f"https://maps.hereapi.com/v3/base/mc/{z}/{x}/{y}/png8?{q}"
        if self.tiles == "tomtom":
            q = urllib.parse.urlencode({"key": self.map_key, "tileSize": 256})
            return f"https://api.tomtom.com/map/1/tile/basic/{'night' if dark else 'main'}/{z}/{x}/{y}.png?{q}"
        return osm_url(z, x, y)

    def _tiles_work(self) -> bool:
        """True when the key may display the provider's own map."""
        try:
            with urllib.request.urlopen(request(self.tile_url("light", 9, 262, 168)), timeout=5) as r:
                return r.headers.get_content_type().startswith("image/")
        except (urllib.error.URLError, TimeoutError, OSError):
            return False


_connections: "OrderedDict[str, Connection]" = OrderedDict()
_by_settings: dict = {}
_lock = threading.Lock()
MAX_CONNECTIONS = 16


def connect(body: dict) -> dict:
    settings = body.get("settings") or {}
    fingerprint = hashlib.sha256(json.dumps(settings, sort_keys=True).encode()).hexdigest()
    with _lock:
        token = _by_settings.get(fingerprint)
        if token in _connections:
            c = _connections[token]
            return {"token": token, "tiles": c.tiles, "provider": c.provider}
    c = Connection(settings)
    token = secrets.token_urlsafe(18)
    with _lock:
        _connections[token] = c
        _by_settings[fingerprint] = token
        while len(_connections) > MAX_CONNECTIONS:
            old, _ = _connections.popitem(last=False)
            for k, v in list(_by_settings.items()):
                if v == old:
                    del _by_settings[k]
    return {"token": token, "tiles": c.tiles, "provider": c.provider}


def connection(token) -> "Connection":
    with _lock:
        c = _connections.get(token or "")
    if c is None:
        raise BadInput("Connect your keys again; the demo server was restarted.")
    return c


# ---------------------------------------------------------------- trip calls

def trip_input(body: dict):
    query = (body.get("query") or "").strip()
    if not query:
        raise BadInput("Type where you want to go.")
    try:
        lat, lng = float(body["lat"]), float(body["lng"])
    except (KeyError, TypeError, ValueError):
        raise BadInput("Set a start point on the map.") from None
    return w.TripInput(query=query, current_location=w.LatLng(lat=lat, lng=lng))


def trip_output(out) -> dict:
    return {
        "status": name(out.status),
        "error_code": name(out.error_code),
        "error_message": out.error_message,
        "geojson": json.loads(out.geojson),
    }


def plan(body: dict) -> dict:
    return trip_output(connection(body.get("token")).app.plan_blocking(trip_input(body)))


def parse(body: dict) -> dict:
    app = connection(body.get("token")).app
    if not hasattr(app, "parse_blocking"):
        raise BadInput("Plan only needs waylingo 0.1.1 or later: pip install -U waylingo")
    out = app.parse_blocking(trip_input(body))
    return {
        "status": name(out.status),
        "error_code": name(out.error_code),
        "error_message": out.error_message,
        "plan": json.loads(out.plan_json) if out.plan_json else None,
        "llm_calls": out.llm_calls,
    }


ROLES = {"origin": w.LocationRole.ORIGIN, "waypoint": w.LocationRole.WAYPOINT,
         "destination": w.LocationRole.DESTINATION}


def choices(raw: dict):
    stops, locations = [], []
    for s in raw.get("stops") or []:
        if s.get("skip"):
            pick = w.StopPick.SKIP()
        else:
            pick = w.StopPick.PLACE(provider_place_id=str(s["place_id"]))
        stops.append(w.StopChoice(stop_request_id=str(s["stop_request_id"]),
                                  occurrence=int(s["occurrence"]), pick=pick))
    for loc in raw.get("locations") or []:
        role = ROLES.get(loc.get("role"))
        if role is None:
            raise BadInput("Unknown place to change.")
        index = loc.get("index")
        locations.append(w.LocationChoice(
            role=role, index=None if index is None else int(index),
            candidate_id=str(loc["candidate_id"])))
    return w.Choices(stops=stops, locations=locations)


def reroute(body: dict) -> dict:
    previous = body.get("geojson")
    if not isinstance(previous, dict):
        raise BadInput("Plan a route first.")
    app = connection(body.get("token")).app
    out = app.reroute_blocking(json.dumps(previous), choices(body.get("choices") or {}))
    return trip_output(out)


def check_model(body: dict) -> dict:
    url = (body.get("url") or "").strip().rstrip("/")
    if not is_local(url):
        return {"ok": False, "message": "Use a localhost address."}
    try:
        with urllib.request.urlopen(url + "/models", timeout=3) as r:
            r.read()
        return {"ok": True, "message": "Model server is running."}
    except Exception:
        return {"ok": False, "message": "Nothing answers at this address yet."}


ROUTES = {"/api/connect": connect, "/api/plan": plan, "/api/parse": parse,
          "/api/reroute": reroute, "/api/check-model": check_model}

# ---------------------------------------------------------------- tiles

_tile_cache: "OrderedDict[tuple, tuple]" = OrderedDict()
_tile_lock = threading.Lock()
MAX_TILES = 3000


def osm_url(z: int, x: int, y: int) -> str:
    return f"https://tile.openstreetmap.org/{z}/{x}/{y}.png"


def tile(token: str, style: str, z: int, x: int, y: int):
    # Before any keys are connected the page shows OpenStreetMap ("public").
    c = None if token == "public" else connection(token)
    source = c.tiles if c else "osm"
    key = (source, c.map_key if c and source != "osm" else "", style, z, x, y)
    with _tile_lock:
        hit = _tile_cache.get(key)
        if hit:
            _tile_cache.move_to_end(key)
            return hit
    url = c.tile_url(style, z, x, y) if c else osm_url(z, x, y)
    with urllib.request.urlopen(request(url), timeout=10) as r:
        data, ctype = r.read(), r.headers.get_content_type()
    with _tile_lock:
        _tile_cache[key] = (data, ctype)
        while len(_tile_cache) > MAX_TILES:
            _tile_cache.popitem(last=False)
    return data, ctype


# ---------------------------------------------------------------- HTTP

class Handler(BaseHTTPRequestHandler):
    server_version = "waylingo-demo"

    def log_message(self, fmt, *args):  # bodies and tile URLs hold keys: log nothing
        pass

    def send(self, status: int, body: bytes, ctype: str, cache: str = "no-store") -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", cache)
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def send_json(self, status: int, data: dict) -> None:
        self.send(status, json.dumps(data).encode(), "application/json")

    def local_host(self) -> bool:
        host = (self.headers.get("Host") or "").rsplit(":", 1)[0].strip("[]")
        return host in LOCAL_HOSTS

    def do_GET(self):
        if not self.local_host():
            return self.send(403, b"forbidden", "text/plain")
        path = urllib.parse.urlsplit(self.path).path
        if path in ("/", "/index.html"):
            return self.send(200, PAGE.read_bytes(), "text/html; charset=utf-8")
        if path.startswith("/static/"):
            target = (STATIC / path[len("/static/"):]).resolve()
            if STATIC in target.parents and target.is_file():
                ctype = mimetypes.guess_type(target.name)[0] or "application/octet-stream"
                return self.send(200, target.read_bytes(), ctype, "max-age=3600")
            return self.send(404, b"not found", "text/plain")
        if path.startswith("/tiles/"):
            parts = path.split("/")  # "", "tiles", token, style, z, x, "y.png"
            try:
                token, style = parts[2], parts[3]
                z, x, y = int(parts[4]), int(parts[5]), int(parts[6].split(".")[0])
                if style not in ("light", "dark") or not 0 <= z <= 19:
                    raise ValueError
                data, ctype = tile(token, style, z, x, y)
                return self.send(200, data, ctype, "max-age=86400")
            except (ValueError, IndexError):
                return self.send(400, b"bad tile", "text/plain")
            except BadInput:
                return self.send(401, b"connect first", "text/plain")
            except (urllib.error.URLError, TimeoutError, OSError):
                return self.send(502, b"tile unavailable", "text/plain")
        self.send(404, b"not found", "text/plain")

    def do_POST(self):
        route = ROUTES.get(self.path)
        if route is None:
            return self.send(404, b"not found", "text/plain")
        # Only this page may call the API: refuse other sites and rebinding hosts.
        origin = self.headers.get("Origin")
        if not self.local_host() or (origin is not None and not is_local(origin)):
            return self.send_json(403, {"error_message": "Requests must come from the demo page."})
        try:
            length = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(min(length, 4_000_000)) or b"{}")
            self.send_json(200, route(body))
        except BadInput as e:
            self.send_json(400, {"error_message": str(e)})
        except w.WaylingoError as e:
            self.send_json(400, {"error_code": type(e).__name__, "error_message": str(e)})
        except Exception as e:  # keep the server up; say what broke
            self.send_json(500, {"error_code": type(e).__name__, "error_message": str(e)})


def main() -> None:
    ap = argparse.ArgumentParser(description="Waylingo demo page on localhost.")
    ap.add_argument("--port", type=int, default=8770)
    args = ap.parse_args()
    print(f"Waylingo {w.version()} demo: http://127.0.0.1:{args.port}", flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
