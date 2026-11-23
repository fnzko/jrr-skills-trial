#!/bin/bash
# usage: caffeinate -i matrix.sh "<variants>" "<tasks>" "<indices>"
# Runs every missing probe, at most 2 at a time, launches staggered by 15 s.
set -eu
B="/Users/facundo/code/wye/jrr-refactor-trial/v3/bin"
L="/Users/facundo/code/wye/jrr-refactor-trial/v3/logs"
cd /Users/Shared
for i in $3; do
  for t in $2; do
    for v in $1; do
      [ -e "$L/$v-$t-$i.jsonl" ] && continue
      while [ "$(jobs -rp | wc -l | tr -d ' ')" -ge 2 ]; do sleep 5; done
      echo "$(date +%T) start $v-$t-$i"
      "$B/probe3.sh" "$v" "$t" "$i" &
      sleep 15
    done
  done
done
wait
echo "$(date +%T) done"
