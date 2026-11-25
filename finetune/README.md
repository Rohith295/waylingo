# Fine-tuning a small parser model

Waylingo's LLM step only turns one sentence into one fixed JSON plan
(`SurfaceIntent`). A small model fine-tuned for that job can run on a laptop or a
phone, with no API key, no per-call cost, and no text leaving the device.

Everything here runs on Apple Silicon with [MLX](https://github.com/ml-explore/mlx-lm).
Generated data, adapters, models, logs and the Python environment are gitignored.

## 1. Generate the dataset

```bash
cargo run -q -p waylingo-parse --example make_dataset            # 40 000 examples
```

Writes `finetune/data/`:

| File | Rows | Purpose |
| :--- | :--- | :--- |
| `train.jsonl` | 36 000 | training |
| `valid.jsonl` | 2 000 | validation loss during training |
| `test.jsonl` | 2 000 | held-out sentences, same place pool as training |
| `test_unseen.jsonl` | 1 000 | only places and brands that never appear in training: detects memorisation |

Each row is `{"messages": [system, user, assistant], "meta": {...}, "saved": [...]}`:

* system: `COMPACT_SYSTEM_PROMPT` ("Convert the trip request to Waylingo JSON.");
* user: the exact message format the engine sends (locale, saved-place names, request);
* assistant: canonical compact JSON, empty fields omitted, or
  `{"error":"not_understood","stops":[]}` for text that is not a trip request.

Labels are built from the same template choices as the sentence, every positive
label must compile with the real plan compiler, and places, brands and free text
are copied verbatim from the sentence. English only; places from Europe, the US and
India. No map provider data is involved.

## 2. Install MLX (once)

```bash
python3 -m venv finetune/.venv
finetune/.venv/bin/pip install "mlx-lm==0.32.2"
```

Gated models (Gemma, FunctionGemma) also need you to accept the license on Hugging
Face and run `hf auth login` yourself.

## 3. Train (LoRA)

```bash
finetune/.venv/bin/mlx_lm.lora --model Qwen/Qwen3-0.6B --train --data finetune/data \
  --fine-tune-type lora --mask-prompt --num-layers 16 --batch-size 8 --iters 2000 \
  --learning-rate 1e-4 --steps-per-report 50 --steps-per-eval 200 --val-batches 40 \
  --save-every 500 --max-seq-length 512 --adapter-path finetune/adapters/qwen3-0.6b
```

Watch `Val loss` against `Train loss`: if validation loss rises while training loss
keeps falling, the model is memorising. Do not pick the checkpoint by validation
loss, though: it is measured on generated sentences. Score each checkpoint on real
human wording and on unseen places (section 4) and keep the one with the best mean
of the two.

## 4. Evaluate

```bash
finetune/.venv/bin/mlx_lm.server --model Qwen/Qwen3-0.6B --adapter-path finetune/adapters/qwen3-0.6b --port 8765
EVAL_BASE_URL=http://127.0.0.1:8765/v1 cargo run -q -p waylingo-reqwest --example eval_parser -- 1000 finetune/data/test.jsonl
EVAL_BASE_URL=http://127.0.0.1:8765/v1 cargo run -q -p waylingo-reqwest --example eval_parser -- 1000 finetune/data/test_unseen.jsonl
```

"Same plan" is the number that matters: the model's JSON compiles to exactly the
plan the label compiles to. A large gap between `test` and `test_unseen` means
memorisation.

## 5. Use it in Waylingo

Point the engine at the local server with the compact prompt (in `.env` or the
environment):

```text
LLM_PROVIDER=openai
LLM_BASE_URL=http://127.0.0.1:8765/v1
LLM_MODEL=default
LLM_MODE=prompt_json
LLM_PROMPT=compact
LLM_API_KEY=local
```

In code: `LlmConfig { mode: LlmMode::PromptJson, prompt: PromptStyle::Compact, .. }`.
