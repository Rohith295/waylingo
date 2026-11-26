"""Waylingo demo: plan a trip from one sentence in your browser, with your own keys.

    pip install waylingo
    python demo/server.py

Then open http://127.0.0.1:8770. Keys are typed into the page and sent only to this
local server, which passes them to Waylingo for the calls you start. Nothing is
written to disk or logged.
"""

import argparse
import hashlib
import json
import threading
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import waylingo as w

PAGE = Path(__file__).with_name("index.html")
LOCAL_HOSTS = {"127.0.0.1", "localhost", "::1"}

# One Waylingo instance per set of settings and keys, reused between requests.
_apps: dict = {}
_apps_lock = threading.Lock()


class BadInput(Exception):
    """A missing or invalid setting, shown on the page."""


def is_local(url: str) -> bool:
    try:
        return urllib.parse.urlsplit(url).hostname in LOCAL_HOSTS
    except ValueError:
        return False


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
        raise BadInput("Paste your LLM key, or switch to the built-in model.")
    if kind == "anthropic":
        return w.LlmBackend.ANTHROPIC(model=model, base_url=None), key
    if kind in ("openai", "gemini"):
        if not model:
            raise BadInput("Enter the model name to use.")
        if kind == "openai":
            return w.LlmBackend.OPEN_AI(model=model, base_url=None), key
        return w.LlmBackend.GEMINI(model=model, base_url=None), key
    raise BadInput("Choose an LLM.")


def app_for(settings: dict):
    mp = settings.get("map") or {}
    provider = mp.get("provider")
    map_key = (mp.get("key") or "").strip()
    if provider not in ("tomtom", "here"):
        raise BadInput("Choose TomTom or HERE.")
    if not map_key:
        raise BadInput(f"Paste your {'TomTom' if provider == 'tomtom' else 'HERE'} key.")
    backend, llm_key = llm_backend(settings.get("llm") or {})
    cache_key = hashlib.sha256(
        json.dumps([provider, map_key, repr(backend), llm_key]).encode()
    ).hexdigest()
    with _apps_lock:
        app = _apps.get(cache_key)
        if app is None:
            map_provider = w.MapProvider.TOMTOM if provider == "tomtom" else w.MapProvider.HERE
            app = w.Waylingo(
                w.WaylingoConfig(map_provider=map_provider, llm=backend),
                map_key=map_key,
                llm_key=llm_key,
            )
            _apps[cache_key] = app
        return app


def trip_input(body: dict):
    query = (body.get("query") or "").strip()
    if not query:
        raise BadInput("Type where you want to go.")
    try:
        lat, lng = float(body["lat"]), float(body["lng"])
    except (KeyError, TypeError, ValueError):
        raise BadInput("Set a start point on the map.") from None
    return w.TripInput(query=query, current_location=w.LatLng(lat=lat, lng=lng))


def name(value):
    return None if value is None else str(value).split(".")[-1]


def plan(body: dict) -> dict:
    out = app_for(body.get("settings") or {}).plan_blocking(trip_input(body))
    return {
        "status": name(out.status),
        "error_code": name(out.error_code),
        "error_message": out.error_message,
        "geojson": json.loads(out.geojson),
    }


def parse(body: dict) -> dict:
    app = app_for(body.get("settings") or {})
    if not hasattr(app, "parse_blocking"):
        raise BadInput("This waylingo package has no plan-only call; upgrade it with pip.")
    out = app.parse_blocking(trip_input(body))
    return {
        "status": name(out.status),
        "error_code": name(out.error_code),
        "error_message": out.error_message,
        "plan": json.loads(out.plan_json) if out.plan_json else None,
        "llm_calls": out.llm_calls,
    }


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


ROUTES = {"/api/plan": plan, "/api/parse": parse, "/api/check-model": check_model}


class Handler(BaseHTTPRequestHandler):
    server_version = "waylingo-demo"

    def log_message(self, fmt, *args):  # request bodies hold keys: log nothing
        pass

    def send(self, status: int, body: bytes, ctype: str) -> None:
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def send_json(self, status: int, data: dict) -> None:
        self.send(status, json.dumps(data).encode(), "application/json")

    def same_origin(self) -> bool:
        # Only this page may call the API: refuse other sites and rebinding hosts.
        host = (self.headers.get("Host") or "").rsplit(":", 1)[0].strip("[]")
        origin = self.headers.get("Origin")
        return host in LOCAL_HOSTS and (origin is None or is_local(origin))

    def do_GET(self):
        if self.path in ("/", "/index.html"):
            self.send(200, PAGE.read_bytes(), "text/html; charset=utf-8")
        else:
            self.send(404, b"not found", "text/plain")

    def do_POST(self):
        route = ROUTES.get(self.path)
        if route is None:
            return self.send(404, b"not found", "text/plain")
        if not self.same_origin():
            return self.send_json(403, {"error_message": "Requests must come from the demo page."})
        try:
            length = int(self.headers.get("Content-Length") or 0)
            body = json.loads(self.rfile.read(min(length, 64_000)) or b"{}")
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
