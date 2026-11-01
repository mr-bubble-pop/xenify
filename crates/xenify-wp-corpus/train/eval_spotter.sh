#!/usr/bin/env bash
# Scores rules + a spotter model on the fixed evaluation sets in
# data/proofing-corpus/eval/sets (samples of the book, broken and AI sets,
# plus the full AI test set and the Wikipedia set).
#   eval_spotter.sh NAME MODEL_DIR [THREADS]  → data/proofing-corpus/eval/NAME-SET.json
# MODEL_DIR "none" scores the rules alone. THRESHOLD (default: the app's, 0.97) sets how sure
# the model must be; SETS limits the sets (e.g. SETS="wiki-test uk-books").
set -e
name=$1 model=$2
export RAYON_NUM_THREADS=${3:-4}
cd "$(dirname "$0")/../../.."
E=data/proofing-corpus/eval B=target/release/xenify-wp-corpus
for x in "ai-test us" "wiki-test us" "uk-books uk" "us-books us" "ai-broken us" "uk-broken uk" "us-broken us"; do
    set -- $x
    [ -n "$SETS" ] && [[ " $SETS " != *" $1 "* ]] && continue
    if [ "$model" = none ]; then
        $B score $E/sets/$1.jsonl -o $E/$name-$1 --dialect $2 | tail -1
    else
        $B score $E/sets/$1.jsonl -o $E/$name-$1 --dialect $2 --using both --spotter-model $model ${THRESHOLD:+--spotter-threshold $THRESHOLD} | tail -1
    fi
done
echo "$name done"
