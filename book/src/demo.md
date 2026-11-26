# Try the demo

The repository has a small web app for trying Waylingo before writing any code. It
runs on your own computer: you paste your own map key, type trips in plain words, and
see the route, the stops and their coordinates on a map. You can then change stops,
pick another match for a place and undo, which is the interaction loop from
[Reroute](reroute.md).

The demo uses the published Python package, so what you see is what an app gets from
`plan`, `reroute` and `parse`.

## Watch it

<video controls muted playsinline preload="metadata" width="100%"
       poster="media/waylingo-demo-poster.jpg" style="border-radius: 8px">
  <source src="media/waylingo-demo.mp4" type="video/mp4">
  <a href="media/waylingo-demo.mp4">Download the video (MP4)</a>.
</video>

A 2-minute recording of the demo with the built-in model and a TomTom key, no sound:

| Time | Scenario |
| ---: | :--- |
| 0:05 | Connect your keys |
| 0:20 | Stops placed by time: fuel after 30 minutes, a McDonald's before arrival |
| 0:35 | Change a stop, skip it, bring it back, undo |
| 0:57 | Not the right place? Another match for a name, and a clear answer when no road reaches it |
| 1:12 | A charger every 200 km on the way to Berlin |
| 1:26 | A waypoint and a lunch stop |
| 1:37 | Parse only: the plan as JSON, no map calls |
| 1:45 | Not a trip: refused before any map call |
| 1:53 | Dark theme at phone size |

Place results in the recording come from TomTom and the map from OpenStreetMap; yours
depend on your provider and the day.

## What you need

* Python 3.9 or later and a checkout of the repository.
* A **TomTom or HERE key**. Both have free tiers:
  [developer.tomtom.com](https://developer.tomtom.com/),
  [platform.here.com](https://platform.here.com/).
* A **language model**, one of:
  * the **built-in model**: Waylingo's [fine-tuned parser](fine-tuning.md), running on
    your computer with [llama.cpp](https://github.com/ggml-org/llama.cpp). No key and no
    cost per request.
  * an Anthropic, OpenAI or Gemini key.

## Start it

```bash
git clone https://github.com/Rohith295/waylingo
cd waylingo
pip install -U waylingo
python demo/server.py
```

Open http://127.0.0.1:8770.

For the built-in model, install llama.cpp (`brew install llama.cpp` on macOS; other
systems: the [llama.cpp install guide](https://github.com/ggml-org/llama.cpp#quick-start))
and start the model in a second terminal. The first start downloads it (1.2 GB) from
[Hugging Face](https://huggingface.co/rangaraju963/waylingo-parser-qwen3-0.6b); later
starts take a few seconds.

```bash
llama-server -hf rangaraju963/waylingo-parser-qwen3-0.6b --jinja --temp 0 --port 8765
```

The model server uses about 2 GB of memory while it runs; stop it with Ctrl+C.

## Connect your keys

The page opens on **Connect your keys**:

1. **Map provider:** choose TomTom or HERE and paste the key.
2. **Language model:** choose **Built-in** (the page shows whether the model server is
   running and checks again every few seconds) or paste an Anthropic, OpenAI or Gemini
   key. OpenAI and Gemini also need a model name.

Then choose **Connect**. The button at the top right shows the connection and opens
these settings again.

## Things to try

| Try | What you see |
| :--- | :--- |
| `Drive to Utrecht, coffee halfway` | One stop at the midpoint of the drive, with how much time it adds |
| `To Rotterdam, fuel after 30 minutes and a McDonald's 15 minutes before we arrive` | Two stops placed by time: one from the start, one before arrival |
| `Drive to Berlin, a fast charger every 200 km` | A repeated stop, one charger per 200 km |
| `Take me to Haarlem via Zandvoort, lunch on the way` | A waypoint and a stop |
| `What's the weather tomorrow?` | A refusal: not a trip, and no map calls |

For each result the panel shows the distance, drive time and stops, then the trip in
order: the start, each stop (how far into the drive, how much time it adds, its
coordinates) and the destination. Click a coordinate to zoom to it. The line under
the title says how long it took and how many language model and map calls it used.

**Change a stop.** Open **Change** on a stop to see its alternatives, numbered like
their pins on the map, and choose **Use**. Or click a hollow pin on the map and choose
**Use this stop**. **Skip this stop** leaves it out; **Bring back** restores it. Each of
these reroutes from the previous result with no language model call and one map call.

**Not the right place?** When a place name matched more than one place (there is a
Rotterdam in New York State too), pick another match. This searches the stops again
along the new route, still without the language model. If no road connects the places,
the page says so (`NO_ROUTE`); use **Undo**.

**Undo and Redo** step through your changes.

**Start point.** The trip starts in Amsterdam. Click the map and choose **Start here**,
or use **My location** (the browser asks first).

**Parse only** runs just the language model and shows the plan it wrote, as JSON. No
map calls are made, so places are still names. This is the `parse` call.

**Light and dark.** The page follows your system setting; the button next to the
connection switches it.

At the bottom, **Plan JSON from the model** and **Full result (GeoJSON)** show the raw
documents, and the GeoJSON can be downloaded.

## How it maps to the API

[`demo/server.py`](https://github.com/Rohith295/waylingo/blob/main/demo/server.py) is a
short, readable example of a Waylingo host:

| Page action | Call |
| :--- | :--- |
| Connect | one `Waylingo(config, map_key, llm_key)` per set of keys, reused for every request |
| Plan route | `plan_blocking(TripInput(query, current_location))` |
| Use, Skip, Bring back, Not the right place? | `reroute_blocking(previous_geojson, Choices(...))` |
| Parse only | `parse_blocking(TripInput(...))` |

The page builds `Choices` from the result itself: a stop's `stop_request_id`,
`occurrence` and an alternative's `provider_place_id`, or a place's `candidate_id`. See
[The result document](result-document.md) for those fields.

## Your keys

Keys go only from the page to `server.py`, which listens on `127.0.0.1` and keeps them
in memory for the calls you start. It writes nothing to disk, logs nothing and refuses
requests from other websites. Map tiles also go through it, so keys never appear in
browser URLs. In the browser, keys last until the tab is closed, unless you turn on
**Remember keys on this browser**; **Forget saved keys** removes them.

The page loads nothing from other websites: its fonts and map library are in
`demo/static`. Map tiles come from HERE with a HERE key, from TomTom when the key
includes map display, and otherwise from OpenStreetMap.

The demo is for your own machine. Do not expose it on a network.

## Troubleshooting

| Message | What to do |
| :--- | :--- |
| `couldn't bind HTTP server socket ... port: 8765` when starting `llama-server` | A model server is already running on that port; use it, or stop it first. To use another port, add `--port 8766` and change **Model address** in the settings |
| `Address already in use` when starting `server.py` | The demo is already running; open http://127.0.0.1:8770, or start another copy with `python demo/server.py --port 8771` |
| "Not running yet" under the built-in model | Start `llama-server` with the command above and wait for it to finish loading |
| The language model is not reachable (`LLM_UNREACHABLE`) | The model server stopped, or the address is wrong |
| The map key was refused (`PROVIDER_AUTH`) | Check the key, and that it has routing and search enabled |
| "Parse only needs waylingo 0.1.1 or later" | `pip install -U waylingo` |
| The map shows OpenStreetMap with a TomTom key | The key does not include TomTom's map display API; routing and search still use TomTom |
| My location does nothing | The browser blocked location access; allow it for 127.0.0.1, or click the map |
