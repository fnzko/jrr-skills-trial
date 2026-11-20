"""usage: blind.py <phase> <run>...   copies each run's crate sources to v2/blind/<phase>/<CODE>/ under a shuffled code"""
import os, random, shutil, sys
V = os.path.expanduser("~/code/wye/jrr-refactor-trial/v2")
phase, runs = sys.argv[1], sys.argv[2:]
rng = random.Random()
codes = rng.sample([a + b for a in "BCDFGHJKLMNPQRSTVWXZ" for b in "AEIOU"], len(runs))
os.makedirs(f"{V}/blind/{phase}", exist_ok=True)
with open(f"{V}/keys/blind-{phase}.tsv", "a") as key:
    for run, code in zip(runs, codes):
        dst = f"{V}/blind/{phase}/{code}"
        os.makedirs(dst)
        for item in ("src", "tests", "Cargo.toml"):
            src = f"{V}/runs/{run}/{item}"
            if os.path.isdir(src):
                shutil.copytree(src, f"{dst}/{item}")
            elif os.path.isfile(src):
                shutil.copy(src, f"{dst}/{item}")
        key.write(f"{code}\t{run}\n")
print("blinded", len(runs), "runs into", f"{V}/blind/{phase}")
