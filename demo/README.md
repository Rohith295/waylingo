# Waylingo demo

A page on your own computer for trying Waylingo with your own keys: type a trip
request, and see the route, the stops and their coordinates on a map, plus the plan
the language model wrote.

You need:

* Python 3.9 or later.
* A **TomTom or HERE key** (required). Both have free tiers:
  [TomTom](https://developer.tomtom.com/), [HERE](https://platform.here.com/).
* A **language model**, one of:
  * the **built-in model**: Waylingo's own fine-tuned parser, running on your computer
    with [llama.cpp](https://github.com/ggml-org/llama.cpp). No key needed.
  * an Anthropic, OpenAI or Gemini key.

## Run it

From a checkout of this repository:

```bash
pip install waylingo
python demo/server.py
```

Open http://127.0.0.1:8770 and connect your keys.

For the built-in model, install llama.cpp (`brew install llama.cpp` on macOS; for other
systems see the [llama.cpp install guide](https://github.com/ggml-org/llama.cpp#quick-start))
and start it in a second terminal. The first start downloads the model (1.2 GB) from
[Hugging Face](https://huggingface.co/rangaraju963/waylingo-parser-qwen3-0.6b):

```bash
llama-server -hf rangaraju963/waylingo-parser-qwen3-0.6b --jinja --temp 0 --port 8765
```

Then choose **Built-in** on the page and use **Check connection**.

## What you can do

* **Plan route** runs the whole trip: the language model reads the request, then the
  map provider finds the places and the route. The panel lists the start, each stop
  (how far into the drive, how much time it adds, its coordinates) and the
  destination. Click a coordinate to zoom to it.
* **Change a stop**: open **Change** to use one of its alternatives or skip it, and
  **Bring back** a skipped stop. Alternatives are also on the map; click one and
  choose **Use this stop**. Each change reroutes from the previous result with no
  language model call and one map call.
* **Did you mean**: when a place name matched more than one place, pick another
  match. This searches the stops again on the new route, still without the language
  model.
* **Undo** steps back through your changes.
* **My location** starts the trip where you are (the browser asks first); or click
  the map to set the start.
* **Plan only** stops after the language model: one call, no map calls. It shows the
  plan as JSON, with places still as names.
* **Light and dark**: the page follows your system setting; the button next to the
  connection status switches it.
* The full result GeoJSON can be viewed and downloaded at the bottom.

## What it loads

Nothing from other websites. The fonts ([Overpass](https://github.com/RedHatOfficial/Overpass),
SIL Open Font License) and [Leaflet](https://leafletjs.com/) (BSD 2-clause) are in
`demo/static` with their licenses. Map tiles come through `server.py`: from HERE when
you use a HERE key, from TomTom when your TomTom key includes map display, and
otherwise from [OpenStreetMap](https://www.openstreetmap.org/copyright). Routing and
search need the internet anyway, since they are your map provider's services.

## Your keys

The page sends keys only to `server.py`, which listens on `127.0.0.1` and passes them
to Waylingo for the calls you start. The server writes nothing to disk, logs nothing,
and refuses requests from other websites. In the browser, keys last until the tab is
closed; turn on **Remember keys on this browser** to keep them in the browser's local
storage, and use **Forget saved keys** to remove them.

The demo is for trying Waylingo on your own machine. Do not expose it on a network.
