//! 決定性 oracle: 同じ操作列は何度実行しても、どの target でも同じ結果になる
//!
//! 1. 別 process で同じ操作列を 2 回走らせ、生き残った key 集合と hit の値が一致する
//! 2. 操作列の結果の FNV-1a digest が定数 (GOLDEN) と一致する (cfg 無し、全 target)
//! 3. seed が違えば結果が変わり、同じ seed なら再現する (seed を引ける版のみ)
//!
//! GOLDEN の再生成:
//!   `ALICE_CACHE_PRINT_GOLDEN=1 cargo test --test determinism golden -- --nocapture`
//! (出力された `GOLDEN=0x...` を下の定数へ写す、操作列 / 容量 / key 範囲を変えた時だけ)

use alice_cache::AliceCache;
use std::process::Command;

const CHILD_ENV: &str = "ALICE_CACHE_DETERMINISM_CHILD";
const CAPACITY: usize = 512;
const OPS: usize = 20_000;
const KEY_SPACE: u64 = 4_000;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv_u64(mut h: u64, v: u64) -> u64 {
    for b in v.to_le_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// 固定 LCG (Knuth MMIX)
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
}

/// 操作列を走らせ (生き残った key 数, hit 数, digest) を返す
/// digest は hit ごとの (操作番号, 返った値) と、最後に残っている key の昇順列を畳んだもの
fn run(cache: &AliceCache<u64, u64>) -> (usize, u64, u64) {
    let mut rng = Lcg(0x5EED_0001);
    let mut h = FNV_OFFSET;
    let mut hits = 0u64;
    for op in 0..OPS as u64 {
        let key = rng.next() % KEY_SPACE;
        if rng.next().is_multiple_of(3) {
            // 値は操作番号: 別の key の値が返れば digest が変わる
            cache.put(key, op);
        } else if let Some(v) = cache.get(&key) {
            hits += 1;
            h = fnv_u64(h, op);
            h = fnv_u64(h, key);
            h = fnv_u64(h, v);
        }
    }
    let mut survivors = 0usize;
    for key in 0..KEY_SPACE {
        if cache.contains(&key) {
            survivors += 1;
            h = fnv_u64(h, key);
        }
    }
    (survivors, hits, h)
}

fn line(r: (usize, u64, u64)) -> String {
    format!("RESULT survivors={} hits={} digest={:#018x}", r.0, r.1, r.2)
}

/// 子 process 側: 結果を 1 行で出す (親が起動した時だけ動く)
#[test]
fn child_emit() {
    if std::env::var(CHILD_ENV).is_err() {
        return;
    }
    let cache = AliceCache::<u64, u64>::new(CAPACITY);
    println!("{}", line(run(&cache)));
}

fn spawn_child() -> String {
    let exe = std::env::current_exe().expect("current_exe");
    let out = Command::new(exe)
        .args(["--exact", "child_emit", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("spawn child");
    assert!(out.status.success(), "child failed: {out:?}");
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    // harness の "test child_emit ... " が同じ行の前に付く
    let found: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.find("RESULT ").map(|i| &l[i..]))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "child must emit exactly one RESULT line: {stdout}"
    );
    found[0].to_string()
}

#[test]
fn separate_processes_agree() {
    let a = spawn_child();
    let b = spawn_child();
    let c = spawn_child();
    assert!(a.contains("survivors=") && !a.contains("survivors=0 "));
    assert_eq!(a, b, "process 1 vs 2");
    assert_eq!(a, c, "process 1 vs 3");
}

const GOLDEN: u64 = 0xcf8f_d59c_0492_257c;
const GOLDEN_SEED_1: u64 = 0xd8e1_03c7_94fa_c963;
const GOLDEN_SEED_2: u64 = 0xb8da_6edf_7a4c_5c07;

#[test]
fn golden() {
    let cache = AliceCache::<u64, u64>::new(CAPACITY);
    let r = run(&cache);
    if std::env::var("ALICE_CACHE_PRINT_GOLDEN").is_ok() {
        println!("GOLDEN={:#018x} survivors={} hits={}", r.2, r.0, r.1);
    }
    // 容量を超える操作列なので追い出しが起きている (空振り防止)
    assert!(r.0 > 0 && r.0 <= CAPACITY, "survivors {}", r.0);
    assert!(r.1 > 0, "no hits");
    assert_eq!(r.2, GOLDEN, "digest changed: {:#018x}", r.2);
}

fn seeded(seed: u64) -> (usize, u64, u64) {
    run(&AliceCache::<u64, u64>::with_seed(CAPACITY, seed))
}

#[test]
fn explicit_seed_reproduces_and_differs() {
    if std::env::var("ALICE_CACHE_PRINT_GOLDEN").is_ok() {
        println!("GOLDEN_SEED_1={:#018x}", seeded(1).2);
        println!("GOLDEN_SEED_2={:#018x}", seeded(2).2);
    }
    // 同じ seed は再現する
    assert_eq!(seeded(1), seeded(1));
    assert_eq!(seeded(2), seeded(2));
    // 違う seed は配置が変わるので結果が変わる
    assert_ne!(seeded(1).2, seeded(2).2);
    // 値を固定する
    assert_eq!(seeded(1).2, GOLDEN_SEED_1);
    assert_eq!(seeded(2).2, GOLDEN_SEED_2);
}

#[test]
fn default_constructor_uses_default_seed() {
    let a = run(&AliceCache::<u64, u64>::new(CAPACITY));
    let b = seeded(alice_cache::DEFAULT_SEED);
    assert_eq!(a, b);
}
