#!/usr/bin/env python3
"""Breaks the shard capacity split one way at a time; tests/shard_capacity.rs must go red.

usage: python3 scripts/capacity_mutants.py
exit 0 if every mutant applied, compiled and made a test report FAILED; exit 1 otherwise.

Run by hand when the capacity split in src/cache.rs or src/shard.rs changes. A mutant that does not
compile or no longer applies counts against the result. A mutant past the time limit is killed with
its process group and counts as red. Each file is restored after its mutant.
"""
import os
import re
import signal
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CACHE = ROOT / "src" / "cache.rs"
SHARD = ROOT / "src" / "shard.rs"
COMMAND = ["cargo", "test", "--test", "shard_capacity"]
FAILED = re.compile(r"^test .* \.\.\. .*FAILED|signal: \d+", re.M)
TIMEOUT_SECS = 300

ONE_MORE = "floor + usize::from(i < with_one_more),"
# (what breaks, file, old text, new text)
MUTANTS = [
    ("every shard rounded up", CACHE, ONE_MORE, "config.capacity.div_ceil(config.num_shards),"),
    ("every shard rounded down", CACHE, ONE_MORE, "floor,"),
    ("one shard too many get one more", CACHE, "usize::from(i < with_one_more)", "usize::from(i <= with_one_more)"),
    ("shard count not lowered", CACHE, "config.num_shards = config.num_shards.min(fits);\n", ""),
    ("shard count lowered too far", CACHE, "config.num_shards = config.num_shards.min(fits);", "config.num_shards = 1;"),
    ("capacity 0 stores a key", SHARD, "if self.capacity == 0 {\n            return;\n        }\n", ""),
    ("capacity 1 treated as 0", SHARD, "if self.capacity == 0 {", "if self.capacity <= 1 {"),
]


def run_tests() -> tuple[int | None, str]:
    p = subprocess.Popen(
        COMMAND, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        start_new_session=True,
    )
    try:
        out, _ = p.communicate(timeout=TIMEOUT_SECS)
        return p.returncode, out
    except subprocess.TimeoutExpired:
        os.killpg(p.pid, signal.SIGKILL)
        out, _ = p.communicate()
        return None, out


def main() -> int:
    if sys.argv[1:]:
        print(__doc__, file=sys.stderr)
        return 2
    bad = []
    for name, path, old, new in MUTANTS:
        original = path.read_text(encoding="utf-8")
        if original.count(old) != 1:
            bad.append(name)
            print(f"NOT APPLIED  {name}", flush=True)
            continue
        try:
            path.write_text(original.replace(old, new), encoding="utf-8")
            rc, output = run_tests()
        finally:
            path.write_text(original, encoding="utf-8")
        if "could not compile" in output:
            bad.append(name)
            print(f"BROKEN       {name}", flush=True)
        elif rc is None:
            print(f"red (killed) {name}", flush=True)
        elif rc != 0 and FAILED.search(output):
            print(f"red          {name}", flush=True)
        else:
            bad.append(name)
            print(f"SURVIVED     {name} (rc {rc})", flush=True)
    print(f"{len(MUTANTS) - len(bad)} of {len(MUTANTS)} red")
    return 1 if bad or not MUTANTS else 0


if __name__ == "__main__":
    sys.exit(main())
