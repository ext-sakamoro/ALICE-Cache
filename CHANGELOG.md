# Changelog

All notable changes to ALICE-Cache will be documented in this file.

## [Unreleased]

### Changed
- Licence: `AGPL-3.0-or-later OR LicenseRef-Commercial` → `LicenseRef-Proprietary`
- CI: production code (`src/`) が OS の乱数・時刻を読まないことを検査する `scripts/entropy_guard.py` (試験 `scripts/test_entropy_guard.py`) を足した `RandomState` の使用 (`::default()` を含む)、`getrandom` `OsRng` `thread_rng` `SystemTime::now` `Instant::now` などを `#[cfg(test)]` の外で見つけると失敗し、読んだ file か pattern が 0 件でも失敗する `clippy.toml` の `disallowed-methods` / `disallowed-types` が同じ対象を型解決で止める CI (actionlint job) と preflight で走る
- CI: 上限の無かった 15 job に `timeout-minutes` を付けた (止まった step は共有 runner を GitHub の上限の 6 時間まで占有しうる) 値は直近の成功した run 10 本の所要時間の最大の 2 倍・中央値の 4 倍の大きい方を 5 分単位に切り上げ、cargo を使う job は最低 15 分 (cache が外れた build のため)、他は最低 10 分 fuzz は 30 分で、`workflow_dispatch` の `duration_seconds` は 1 から 1200 の整数 (先頭の 0 は不可、0 は libFuzzer で無制限になる) でなければ理由を出して止める (それより長い run は build と合わせて上限を超え途中で切れる) 入力は script に埋め込まず `env` で渡す `scripts/workflow_timeouts.py` (試験 `scripts/test_workflow_timeouts.py`、ALICE-LOL と byte 一致) が全 job の上限を検査し、job を 1 つも読めなければ失敗する CI (actionlint job) と preflight で走る
- CI: どの workflow も `ci/**` の branch への push でも走るようにした (`push.branches` に `ci/**` を足した) main に入れる前の commit をその branch で検証するため これまでは main への push と pull request でしか走らず、branch の commit には CI の結果が無かった ci.yml には concurrency を足し、同じ branch に新しい commit が来たら古い run を止める (main では commit ごとに別の group で止めない) path の条件がある workflow (fuzz / security-audit) は従来どおり対象の path が変わった時だけ走る、その 2 つにも ci.yml と同じ concurrency (main は commit ごとの独立 group) を足した
- License: `AGPL-3.0-or-later` → `AGPL-3.0-or-later OR LicenseRef-Commercial` (2026-09-27)

### Fixed
- hash の seed が OS の乱数だった (`ahash::RandomState::new()`) 同じ操作列でも実行ごとに key の shard 配置が変わり、どの key が追い出されずに残るか・hit が返す値が実行ごとに変わっていた 固定 seed の hash (`src/seed.rs`、乱数・時刻を読まず、入力を little-endian の byte 列として畳むので target に依らず同じ値) に置き換え、`AliceCache::new` / `with_config` は既定 seed (`DEFAULT_SEED`) を使う shard 内の lookup 表 (`hashbrown`) も同じ固定 seed にした 既定の構築でも追い出し結果は以前と変わる (実行間で固定になる)
- 同じ操作列を別 process で走らせて生き残った key 集合と hit の値が一致すること、結果の FNV-1a digest が全 target で定数と一致することを `tests/determinism.rs` で固定した

### Fixed
- shard 内の lookup 表が shard 選択と同じ hash 関数を使っていたため、1 shard の key の下位 bit が全部同じになり hashbrown の探索が偏って put が約 2.5 倍、get が約 2 倍遅くなっていた 表の関数を cache の seed から別に導出 (`SeededState::table_for`、`DEFAULT_SEED` でも cache と別の関数) し、結果 (生き残る key / hit の値 / digest) は変えずに戻した `tests/determinism.rs` の digest は不変 shard の key が表の下位 bit に散ることを cache の試験で固定した
- 固定の既定 seed は公開値なので、key を攻撃者が選べる場合は衝突する key を作れる (以前の乱数 seed ではできなかった) 信頼できない key を受ける呼び出し側は秘密の seed を `with_seed` で渡す (`with_seed` の rustdoc に記載)

### Changed
- CI: test job を `ubuntu-latest` と `ubuntu-24.04-arm` の matrix にした golden digest を別 CPU 構成でも固定する

### Added
- `AliceCache::with_seed(capacity, seed)` / `AliceCache::with_config_and_seed(config, seed)` seed を明示でき、同じ seed と同じ操作列は常に同じ結果になる
- `SeededState` / `SeededHasher` / `DEFAULT_SEED` を公開

### Changed
- `ahash` 依存を外した (`std` feature の `ahash/std` `ahash/runtime-rng` も不要になった) `hashbrown` は既定の hasher を使わない構成 (`default-features = false`) にした

### Fixed
- `no_std` が 64-bit atomic の無い target (Cortex-M4 `thumbv7em-none-eabihf`) で `core::sync::atomic::AtomicU64` 不能だった → 統計 counter / oracle の `AtomicU64` を `portable-atomic` (fallback feature) に置換、CI の no_std job に thumbv7em build を追加 (strict-eval 検査 1、2026-09-17)

### Changed

- The optional `alice-crypto` / `alice-analytics` bridges are plain crates.io
  dependencies (`path = "../ALICE-*"` removed); CI no longer builds
  manifest-only sibling stubs, so `cargo audit` / `cargo deny` see the real
  dependency tree.

### Fixed
- **`no_std` が bare-metal で偽だった** — 非 optional の `parking_lot` (std 必須) と `ahash::RandomState::new()` (runtime RNG) により `aarch64-unknown-none` では `can't find crate for std` (host の `--no-default-features` は host std を暗黙 link して見かけ green) `parking_lot` を `std` feature 限定 optional に、`no_std` は `spin::Mutex` (`shard.rs` で cfg 切替) + `ahash` `compile-time-rng` `AtomicU64` 統計 counter のため 64-bit atomics を持つ target が前提 (32-bit MCU は対象外、lib doc に明記)

### Changed
- CI: clippy を `--all-targets` (benches / examples) に、`no_std` job (host rlib + bare-metal `aarch64-unknown-none` + clippy-driver wrapper)、`feature-powerset` (cargo-hack、std 固定 depth 2)、sibling stub を `alice-stubs` action に統一、rust-cache
- deps: `parking_lot` optional (`std`)、`spin` 0.9 追加 (`no_std` のみ)、`ahash` `default-features = false` + `compile-time-rng` (`std` で `std` / `runtime-rng` を有効化、std build の挙動は不変)

## [0.2.0] - 2026-02-23

### Added
- `cache` — `AliceCache` with 256 shards, slab allocation, sampled eviction, `CacheConfig`, `CacheStats`
- `shard` — Per-shard `parking_lot::Mutex`, dense `Vec<Entry>` storage, O(1) random sampling
- `oracle` — Lock-free `AtomicU8` frequency sketch for predictive prefetch
- `sketch` — Count-Min Sketch with halving decay for frequency estimation
- `jump_hash` — O(1) consistent hashing for distributed key routing (`jump_hash`, `jump_hash_bytes`, `jump_hash_u128`)
- `analytics_bridge` — (feature `analytics`) Cache hit/miss metrics and hot key tracking via ALICE-Analytics
- `crypto_bridge` — (feature `crypto`) Signed cache entries for tamper prevention via ALICE-Crypto
- `python` — (feature `pyo3`) Python bindings
- `no_std` support with `alloc` fallback
- 86 unit tests + 1 doc-test
- CI/CD via GitHub Actions (test, clippy, fmt, doc)
- `#[must_use]` on all public value-returning functions
- clippy pedantic: 0 warnings
