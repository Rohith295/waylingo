#!/usr/bin/env bash
# LoRA fine-tune with automatic resume. The macOS GPU watchdog ("Impacting
# Interactivity") can stop long MLX runs; this restarts from the last checkpoint with
# the remaining steps until all steps are done.
#
#   finetune/train.sh <model> <data_dir> <adapter_dir> <total_steps> [num_layers]
#   finetune/train.sh Qwen/Qwen3-0.6B finetune/data2 finetune/adapters/qwen3-0.6b-v2 18000 28
set -u
cd "$(dirname "$0")/.."
MODEL=$1 DATA=$2 OUT=$3 TOTAL=$4 LAYERS=${5:-16}
SAVE_EVERY=1000
VENV=finetune/.venv/bin
LOG=finetune/logs/$(basename "$OUT").log
mkdir -p "$OUT" finetune/logs
export HF_HUB_DISABLE_TELEMETRY=1 NO_COLOR=1

done_steps=0
for attempt in 1 2 3 4 5 6; do
  remaining=$((TOTAL - done_steps))
  [ "$remaining" -le 0 ] && break
  resume=()
  if [ "$done_steps" -gt 0 ]; then
    resume=(--resume-adapter-file "$OUT/adapters.safetensors")
  fi
  echo "attempt $attempt: from step $done_steps, $remaining steps to go" | tee -a "$LOG"
  # Checkpoints of this attempt go to a separate folder so step numbers do not clash.
  run="$OUT/run$attempt"
  mkdir -p "$run"
  $VENV/mlx_lm.lora --model "$MODEL" --train --data "$DATA" --fine-tune-type lora \
    --mask-prompt --num-layers "$LAYERS" --batch-size 2 --iters "$remaining" \
    --learning-rate 1e-4 --steps-per-report 200 --steps-per-eval 1000 --val-batches 100 \
    --save-every "$SAVE_EVERY" --max-seq-length 512 --adapter-path "$run" ${resume[@]+"${resume[@]}"} \
    >> "$LOG" 2>&1
  rc=$?
  cp "$run/adapter_config.json" "$OUT/" 2>/dev/null
  # Latest weights of this attempt: the final file, or the newest checkpoint.
  last=$(ls "$run"/[0-9]*_adapters.safetensors 2>/dev/null | sort | tail -1)
  if [ "$rc" -eq 0 ] && [ -f "$run/adapters.safetensors" ]; then
    cp "$run/adapters.safetensors" "$OUT/adapters.safetensors"
    done_steps=$TOTAL
  elif [ -n "$last" ]; then
    cp "$last" "$OUT/adapters.safetensors"
    step=$(basename "$last" | sed 's/_adapters.safetensors//; s/^0*//')
    done_steps=$((done_steps + step))
  fi
  echo "attempt $attempt ended rc=$rc, total steps done $done_steps" | tee -a "$LOG"
done
echo "exit=$([ "$done_steps" -ge "$TOTAL" ] && echo 0 || echo 1) steps=$done_steps" | tee -a "$LOG"
