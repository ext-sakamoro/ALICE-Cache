#!/usr/bin/env python3
"""Python-side tests for the `pyo3` feature (src/python.rs).

Builds the crate as a Python extension for the interpreter running this script
(`cargo rustc --lib --crate-type cdylib --features pyo3`, PYO3_PYTHON = this
interpreter), imports it as `alice_cache`, checks that every name the module registers is
exported, and checks the jump consistent hash against the Lamping-Veach
algorithm and FNV-1a 64 written out below (not computed by the crate).
Needs numpy (the batch function returns a numpy array).

The extension is built in the debug profile on purpose: debug assertions make
pyo3 abort on a misaligned pyclass (contents aligned above CPython's 16 bytes),
which a release build would run as silent undefined behaviour (src/python.rs
also asserts the alignment of every pyclass at compile time).

Usage: python3 scripts/test_python_bindings.py
Exit 1 when the build fails, when a test fails or when no test ran.
"""

from __future__ import annotations

import importlib.util
import math
import os
import pathlib
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent.parent
MODULE = None  # set by load_module()


def build_extension() -> pathlib.Path:
    env = dict(os.environ)
    env["PYO3_PYTHON"] = sys.executable
    target_dir = pathlib.Path(env.get("CARGO_TARGET_DIR", ROOT / "target"))
    cmd = ["cargo", "rustc", "--lib", "--crate-type", "cdylib", "--features", "pyo3"]
    profile = "debug"
    if sys.platform == "darwin":
        # the interpreter provides the Python symbols at import time; maturin passes the same
        # two arguments (a dependency's build script cannot add link arguments to this cdylib)
        cmd += ["--", "-C", "link-arg=-undefined", "-C", "link-arg=dynamic_lookup"]
    print("+", " ".join(cmd), f"(PYO3_PYTHON={sys.executable})", flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)
    out = target_dir / profile
    for name in ("libalice_cache.so", "libalice_cache.dylib", "alice_cache.dll"):
        if (out / name).is_file():
            return out / name
    raise SystemExit(f"no extension library found in {out}")


def load_module(lib: pathlib.Path, workdir: pathlib.Path):
    suffix = ".pyd" if sys.platform == "win32" else ".so"
    dest = workdir / f"alice_cache{suffix}"
    shutil.copyfile(lib, dest)
    spec = importlib.util.spec_from_file_location("alice_cache", dest)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {dest}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


EXPORTED = [
    "AliceCache", "CountMinSketch", "SharedOracle",
    "jump_consistent_hash", "jump_hash_bytes", "jump_hash_batch",
]

MASK64 = (1 << 64) - 1


def jump_reference(key: int, buckets: int) -> int:
    """Lamping and Veach, "A Fast, Minimal Memory, Consistent Hash Algorithm" (2014), figure 1."""
    if buckets <= 0:
        return 0
    b, j = -1, 0
    while j < buckets:
        b = j
        key = (key * 2862933555777941757 + 1) & MASK64
        j = int((b + 1) * (float(1 << 31) / float((key >> 33) + 1)))
    return b


def fnv1a64(data: bytes) -> int:
    h = 0xCBF29CE484222325
    for x in data:
        h ^= x
        h = (h * 0x100000001B3) & MASK64
    return h


KEYS = [0, 1, 2, 3, 0xDEADBEEF, 0x0123456789ABCDEF, MASK64, 1 << 63] + [
    (i * 0x9E3779B97F4A7C15) & MASK64 for i in range(1, 200)
]
BUCKETS = [1, 2, 3, 7, 10, 100, 1000, 65536, 2147483647]


class ExportTests(unittest.TestCase):
    def test_every_registered_name_is_exported(self):
        self.assertEqual([n for n in EXPORTED if not hasattr(MODULE, n)], [])


class JumpHashTests(unittest.TestCase):
    def test_matches_the_published_algorithm(self):
        for n in BUCKETS:
            for k in KEYS:
                self.assertEqual(MODULE.jump_consistent_hash(k, n), jump_reference(k, n), (k, n))

    def test_non_positive_bucket_counts_give_zero(self):
        for n in (0, -1, -2147483648):
            self.assertEqual(MODULE.jump_consistent_hash(12345, n), 0)

    def test_bytes_key_is_fnv1a_then_jump(self):
        for data in (b"", b"a", b"alice", bytes(range(256))):
            for n in (1, 10, 1000):
                self.assertEqual(MODULE.jump_hash_bytes(data, n), jump_reference(fnv1a64(data), n), (data[:8], n))

    def test_batch_equals_the_scalar_function(self):
        import numpy as np

        keys = np.array(KEYS, dtype=np.uint64)
        out = MODULE.jump_hash_batch(keys, 1000)
        self.assertEqual(out.dtype, np.int32)
        self.assertEqual(out.tolist(), [jump_reference(k, 1000) for k in KEYS])


class CacheTests(unittest.TestCase):
    def test_put_get_remove(self):
        c = MODULE.AliceCache(16)
        self.assertEqual(c.capacity, 16)
        self.assertTrue(c.is_empty)
        self.assertIsNone(c.get("a"))
        c.put("a", b"xyz")
        self.assertEqual(bytes(c.get("a")), b"xyz")
        self.assertIn("a", c)
        self.assertEqual(len(c), 1)
        self.assertEqual(bytes(c.remove("a")), b"xyz")
        self.assertNotIn("a", c)
        self.assertIsNone(c.remove("a"))

    def test_single_node_owns_every_key(self):
        c = MODULE.AliceCache.with_config(capacity=8, num_nodes=1, node_id=0)
        for key in ("a", "b", "longer key"):
            self.assertEqual(c.owner_node(key), 0)
            self.assertTrue(c.is_local_owner(key))


class SketchTests(unittest.TestCase):
    def test_estimate_is_at_least_the_true_count(self):
        s = MODULE.CountMinSketch()
        for _ in range(5):
            s.add(42)
        s.add(7)
        self.assertGreaterEqual(s.estimate(42), 5)
        self.assertGreaterEqual(s.estimate(7), 1)
        self.assertEqual(s.total(), 6)
        s.clear()
        self.assertEqual(s.total(), 0)
        self.assertEqual(s.estimate(42), 0)


def main() -> int:
    global MODULE
    print(f"python {sys.version.split()[0]} ({sys.executable})", flush=True)
    lib = build_extension()
    with tempfile.TemporaryDirectory() as tmp:
        MODULE = load_module(lib, pathlib.Path(tmp))
        suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
        result = unittest.TextTestRunner(verbosity=2).run(suite)
    if result.testsRun == 0:
        print("no test ran", file=sys.stderr)
        return 1
    print(f"python bindings: {result.testsRun} tests, {len(result.failures)} failures, {len(result.errors)} errors")
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    sys.exit(main())
