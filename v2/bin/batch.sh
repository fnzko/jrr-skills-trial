#!/bin/bash
# usage: batch.sh <list-file> <concurrency>   lines: run arm task model [baseline-dir]
R="$HOME/code/wye/jrr-refactor-trial"; V="$R/v2"
list="$1"; K="${2:-4}"
while read -r run arm task model base; do
  [ -z "$run" ] && continue
  while [ "$(jobs -r | wc -l)" -ge "$K" ]; do sleep 5; done
  ( "$V/bin/run2.sh" "$run" "$arm" "$task" "$model" $base; "$V/bin/eval2.sh" "$run" "$task" ) &
  printf '%s\t%s\t%s\t%s\n' "$run" "$arm" "$task" "$model" >> "$V/keys/runs.tsv"
  sleep 20
done < "$list"
wait
echo "batch finished: $list"
