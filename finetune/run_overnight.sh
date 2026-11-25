#!/usr/bin/env bash
# Unattended pipeline: wait for the running Qwen training, train FunctionGemma,
# then evaluate both on test.jsonl and test_unseen.jsonl. Results go to
# finetune/results/. Run under `caffeinate -i` so the Mac does not idle-sleep.
set -u
cd "$(dirname "$0")/.."
export HF_HUB_DISABLE_TELEMETRY=1 NO_COLOR=1
VENV=finetune/.venv/bin
LOG=finetune/logs/overnight.log
RES=finetune/results
EVAL=target/debug/examples/eval_parser
# 8080 is often taken (OrbStack, Docker); use a less common port.
PORT=${PORT:-8765}
mkdir -p "$RES" finetune/logs
log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" | tee -a "$LOG"; }

# Build the evaluator up front so a build problem shows early.
cargo build -q -p waylingo-reqwest --example eval_parser >>"$LOG" 2>&1 || log "WARN: eval build failed"

if [ "${EVAL_ONLY:-0}" != 1 ]; then
# 1. Wait for the Qwen run that is already training.
log "waiting for Qwen3-0.6B training to finish"
until grep -q "^exit=" finetune/logs/qwen3-0.6b.log 2>/dev/null; do sleep 30; done
log "Qwen3-0.6B training finished: $(grep '^exit=' finetune/logs/qwen3-0.6b.log | tail -1)"

# 2. Train FunctionGemma: batch 4 first; fall back to batch 2 if the macOS GPU
#    watchdog stops it ("Impacting Interactivity").
train_gemma() {
  local batch=$1 iters=$2
  rm -rf finetune/adapters/functiongemma-270m
  mkdir -p finetune/adapters/functiongemma-270m
  log "FunctionGemma training: batch $batch, $iters steps"
  $VENV/mlx_lm.lora --model google/functiongemma-270m-it --train --data finetune/data \
    --fine-tune-type lora --mask-prompt --num-layers 16 --batch-size "$batch" --iters "$iters" \
    --learning-rate 1e-4 --steps-per-report 200 --steps-per-eval 800 --val-batches 100 \
    --save-every 1600 --max-seq-length 512 --adapter-path finetune/adapters/functiongemma-270m \
    > finetune/logs/functiongemma-270m.log 2>&1
  local rc=$?
  echo "exit=$rc" >> finetune/logs/functiongemma-270m.log
  return $rc
}
if ! train_gemma 4 4500; then
  if grep -q "Impacting Interactivity" finetune/logs/functiongemma-270m.log; then
    log "batch 4 hit the GPU watchdog; retrying at batch 2"
    train_gemma 2 8000 || log "FunctionGemma training failed at batch 2 too"
  else
    log "FunctionGemma training failed (see finetune/logs/functiongemma-270m.log)"
  fi
fi
log "FunctionGemma training finished: $(grep '^exit=' finetune/logs/functiongemma-270m.log | tail -1)"
fi

# 3. Evaluate a model with its adapters on both test sets.
evaluate() {
  local name=$1 model=$2 adapters=$3
  if [ ! -f "$adapters/adapters.safetensors" ]; then
    log "skip eval $name: no adapters.safetensors"
    return
  fi
  if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    log "FAIL eval $name: port $PORT is already in use"
    return
  fi
  log "eval $name: starting server on port $PORT"
  $VENV/mlx_lm.server --model "$model" --adapter-path "$adapters" --host 127.0.0.1 --port "$PORT" \
    > "finetune/logs/server-$name.log" 2>&1 &
  local pid=$!
  # Ready only when our server answers with an OpenAI-style model list (the port was free before we started it).
  local ready=0
  for _ in $(seq 1 120); do
    if curl -sf "http://127.0.0.1:$PORT/v1/models" | grep -q "\"object\""; then ready=1; break; fi
    kill -0 "$pid" 2>/dev/null || break
    sleep 5
  done
  if [ "$ready" != 1 ]; then
    log "FAIL eval $name: server did not come up (see finetune/logs/server-$name.log)"
    kill "$pid" 2>/dev/null
    return
  fi
  # One warm-up request so model load time is not counted as latency.
  curl -s -o /dev/null "http://127.0.0.1:$PORT/v1/chat/completions" -H 'Content-Type: application/json' \
    -d '{"model":"default_model","messages":[{"role":"user","content":"hi"}],"max_tokens":4}' 
  # The server answers with the model it loaded (plus adapters) for "default_model".
  for set in test test_unseen; do
    log "eval $name on $set"
    EVAL_BASE_URL=http://127.0.0.1:$PORT/v1 EVAL_MODEL=default_model \
      "$EVAL" 1000 "finetune/data/$set.jsonl" > "$RES/$name-$set.txt" 2>&1
    grep "^TOTAL" "$RES/$name-$set.txt" | sed "s/^/  $name $set: /" | tee -a "$LOG"
  done
  kill "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  sleep 5
}
evaluate qwen3-0.6b Qwen/Qwen3-0.6B finetune/adapters/qwen3-0.6b
evaluate functiongemma-270m google/functiongemma-270m-it finetune/adapters/functiongemma-270m

# 4. One summary file.
{
  echo "Waylingo parser fine-tuning results ($(date '+%Y-%m-%d %H:%M'))"
  echo
  for name in qwen3-0.6b functiongemma-270m; do
    echo "== $name"
    echo "validation loss during training:"
    tr '\r' '\n' < "finetune/logs/$name.log" | perl -pe 's/\e\[[0-9;]*m//g' | grep -E ' val ' | sed 's/^/  /'
    for set in test test_unseen; do
      [ -f "$RES/$name-$set.txt" ] || continue
      echo "$set:"
      grep -E "^(TOTAL|anchor|region|stops|latency)" "$RES/$name-$set.txt" | sed 's/^/  /'
    done
    echo
  done
} > "$RES/SUMMARY.txt"
log "done: see $RES/SUMMARY.txt"
