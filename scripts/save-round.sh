#!/bin/sh
# save-round.sh <round> <dir>: saves the engine's round view and proof bundle
# for <round> as <dir>/round-<round>.round.json and .proof.json.
set -eu
round="$1"; dir="$2"; engine="${ZEEK_ENGINE_URL:-http://127.0.0.1:8787}"
mkdir -p "$dir"
curl -sf "$engine/v1/rounds/$round" > "$dir/round-$round.round.json"
curl -sf "$engine/v1/rounds/$round/proof" > "$dir/round-$round.proof.json"
python3 - "$dir/round-$round.round.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); s = d.get("settled") or {}
print("round", d.get("round"), "treasure", s.get("treasure_tile"), "pot", s.get("pot"),
      "paid", s.get("total_paid"), "winners", s.get("winners"), "cache",
      s.get("previous_cache"), "->", s.get("new_cache"), "plan", s.get("plan_status"),
      "payout", (s.get("payout") or {}).get("txid"))
PY
