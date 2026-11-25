# Running the parser on a phone

The [fine-tuned parser](fine-tuning.md) is small enough to run inside an Android or iOS
app, so a trip request never leaves the phone and needs no LLM key. **Waylingo cannot
do this yet**: today the bindings call the LLM over HTTP. This chapter records which
on-device runtime to use, what it costs, and how it will connect to Waylingo. It is
based on research done in November 2026; versions move quickly, so check the links.

## The runtime: llama.cpp

Use [llama.cpp](https://github.com/ggml-org/llama.cpp) with the model's GGUF file on
both platforms.

* **One model file for iOS and Android.** The GGUF on Hugging Face has already been
  scored with llama.cpp and matches the original weights.
* **It takes the exact prompt.** Waylingo builds the prompt the model was trained on;
  llama.cpp runs it as given, without applying its own chat template.
* **Greedy decoding and JSON grammars.** The model must decode greedily. llama.cpp can
  also restrict output to a grammar (GBNF, or a JSON schema converted to one), which
  guarantees well-formed JSON.
* **Packaging.** Every release has a prebuilt iOS XCFramework that Swift Package Manager
  can use, with a checksum
  ([docs](https://github.com/ggml-org/llama.cpp/blob/master/docs/xcframework.md); iOS
  16.4 or later). On Android there is no official Maven package: build the library with
  the NDK ([docs](https://github.com/ggml-org/llama.cpp/blob/master/docs/android.md)),
  the same toolchain Waylingo's own Android build uses.
* MIT licence.

Other runtimes and why they are second choice:

| Runtime | Platforms | Why not first |
| :--- | :--- | :--- |
| [MLX Swift](https://github.com/ml-explore/mlx-swift-lm) | iOS 17+, macOS | A good iOS-only option, with JSON-schema guided generation; nothing for Android |
| [LiteRT-LM](https://github.com/google-ai-edge/LiteRT-LM) | Android (stable), iOS (preview) | Bakes the chat template into the model file and documents no raw-prompt input or arbitrary JSON schema |
| [ExecuTorch](https://github.com/pytorch/executorch) | Android, iOS 17+ | Needs a separate model export; no constrained JSON output found |
| [MLC LLM](https://github.com/mlc-ai/mlc-llm) | Android, iOS | No stable release; heavy compile step |
| Apple Foundation Models with Core AI | iOS 27+ | Very new; the framework applies its own template |
| MediaPipe LLM Inference | — | Retired in favour of LiteRT-LM |
| ONNX Runtime GenAI | — | Preview; no mobile packages |

## Size and quantisation

| GGUF | Size | Notes |
| :--- | ---: | :--- |
| bf16 (published) | 1.2 GB | Too large for most apps |
| Q8_0 | about 640 MB | Recommended: 8-bit usually costs under 1% accuracy |
| Q4_K_M | about 400 MB | Only if it scores within about a point of Q8_0 |

Small models lose more from 4-bit quantisation than large ones; for Qwen3-0.6B one
published measurement shows perplexity rising 18% at mixed 4/8-bit
([Apple coreai-models](https://github.com/apple/coreai-models/blob/main/models/qwen3/README.md)).
Before shipping a quantised file, run the same evaluation sets on it.

Memory: about 0.5 to 0.8 GB while running, including the attention cache for a short
request.

## Speed

No one has published phone numbers for this exact model on llama.cpp yet. From
published figures for similar small models, expect about **2 to 4 seconds** per request
on 2023-or-newer flagship phones and **5 to 10 seconds** on mid-range or older ones.
Most of the time is spent generating the JSON, so shorter output is faster.

## Getting the model onto the phone

Keep the model out of the app binary itself:

* **Android:** a fast-follow
  [Play Asset Delivery](https://developer.android.com/guide/playcore/asset-delivery)
  pack (packs can be up to 1.5 GB).
* **iOS 26 and later:** Apple-hosted
  [Background Assets](https://developer.apple.com/documentation/backgroundassets/assetpackmanager);
  on older versions, a download on first launch.
* **Downloading from Hugging Face:** pin a commit (`/resolve/<commit>/…`, not `main`),
  check the file's SHA-256 against a value built into the app, resume interrupted
  downloads, and ask before downloading over mobile data (both stores warn above
  200 MB).

## How it will connect to Waylingo

The plan is a callback that the app implements: Waylingo builds the exact prompt and
calls the app's local runtime, which returns the generated text. The rest (validation,
name check, repairs, routing) stays in Waylingo.

```rust
// Implemented in Swift or Kotlin, called by Waylingo.
#[uniffi::export(with_foreign)]
#[async_trait::async_trait]
pub trait LocalLlm: Send + Sync {
    /// `prompt` is complete and already templated; decode greedily.
    async fn complete(&self, prompt: String, max_tokens: u32, grammar: Option<String>)
        -> Result<String, LlmError>;
}
```

```kotlin
class LlamaLocalLlm(private val llama: LlamaEngine) : LocalLlm {
    override suspend fun complete(prompt: String, maxTokens: UInt, grammar: String?) =
        withContext(Dispatchers.Default) { llama.generate(prompt, maxTokens.toInt(), grammar) }
}
```

The app loads the model once and runs one request at a time. This is not built yet;
the sketch shows the intended shape, not a working API.
