# Changelog

All notable changes to ALICE-Cache will be documented in this file.

## [Unreleased]

### Changed
- CI: 上限の無かった 15 job に `timeout-minutes` を付けた (止まった step は共有 runner を GitHub の上限の 6 時間まで占有しうる) 値は直近の成功した run 10 本の所要時間の最大の 2 倍・中央値の 4 倍の大きい方を 5 分単位に切り上げ、cargo を使う job は最低 15 分 (cache が外れた build のため)、他は最低 10 分 fuzz は 30 分で、`workflow_dispatch` の `duration_seconds` は 1 から 1200 の整数 (先頭の 0 は不可、0 は libFuzzer で無制限になる) でなければ理由を出して止める (それより長い run は build と合わせて上限を超え途中で切れる) 入力は script に埋め込まず `env` で渡す `scripts/workflow_timeouts.py` (試験 `scripts/test_workflow_timeouts.py`、ALICE-LOL と byte 一致) が全 job の上限を検査し、job を 1 つも読めなければ失敗する CI (actionlint job) と preflight で走る
- CI: どの workflow も `ci/**` の branch への push でも走るようにした (`push.branches` に `ci/**` を足した) main に入れる前の commit をその branch で検証するため これまでは main への push と pull request でしか走らず、branch の commit には CI の結果が無かった ci.yml には concurrency を足し、同じ branch に新しい commit が来たら古い run を止める (main では commit ごとに別の group で止めない) path の条件がある workflow (fuzz / security-audit) は従来どおり対象の path が変わった時だけ走る
- **License: `AGPL-3.0-or-later` → `AGPL-3.0-or-later OR LicenseRef-Commercial` (dual-licensed、2026-09-27)** AGPL 側の条件は変更なし (既存 AGPL 利用者への影響ゼロ)、商用という選択肢が追加されただけ SPDX が AGPL 単独だと cargo-deny / FOSSA / SBOM に「商用オプションなし」と見えるため宣言を dual に 変更点: SPDX / `LICENSE` → `LICENSE-AGPL` / `LICENSE-COMMERCIAL.md` (商用トリガー 6 条件 = クローズド製品・商用 SaaS・エッジ / ファームウェア配布・plugin 再配布・プラットフォーム NDA・保証、社内利用は AGPL 側で無償と明記) / README の選択肢表 商用窓口は法人 `contact@extoria.co.jp`

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
