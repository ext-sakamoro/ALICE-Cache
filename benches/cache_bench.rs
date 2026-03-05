use alice_cache::{jump_hash, AliceCache, Sketch4K};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

fn bench_cache_put(c: &mut Criterion) {
    let cache: AliceCache<u64, u64> = AliceCache::new(10000);

    c.bench_function("cache_put", |b| {
        let mut key = 0u64;
        b.iter(|| {
            key = key.wrapping_add(1);
            cache.put(black_box(key), black_box(key * 10));
        });
    });
}

fn bench_cache_get_hit(c: &mut Criterion) {
    let cache: AliceCache<u64, u64> = AliceCache::new(10000);
    for i in 0..10000u64 {
        cache.put(i, i * 10);
    }

    c.bench_function("cache_get_hit", |b| {
        let mut key = 0u64;
        b.iter(|| {
            key = (key + 1) % 10000;
            cache.get(black_box(&key))
        });
    });
}

fn bench_cache_get_miss(c: &mut Criterion) {
    let cache: AliceCache<u64, u64> = AliceCache::new(1000);
    for i in 0..1000u64 {
        cache.put(i, i);
    }

    c.bench_function("cache_get_miss", |b| {
        let mut key = 100_000u64;
        b.iter(|| {
            key = key.wrapping_add(1);
            cache.get(black_box(&key))
        });
    });
}

fn bench_jump_hash(c: &mut Criterion) {
    let mut group = c.benchmark_group("jump_hash");

    for num_buckets in [4, 16, 64, 256] {
        group.bench_with_input(
            BenchmarkId::new("buckets", num_buckets),
            &num_buckets,
            |b, &n| {
                let mut key = 0u64;
                b.iter(|| {
                    key = key.wrapping_add(1);
                    jump_hash(black_box(key), n)
                });
            },
        );
    }
    group.finish();
}

fn bench_count_min_sketch(c: &mut Criterion) {
    let mut sketch = Sketch4K::new();

    c.bench_function("cms_add", |b| {
        let mut key = 0u64;
        b.iter(|| {
            key = key.wrapping_add(1);
            sketch.add(black_box(key));
        });
    });

    c.bench_function("cms_estimate", |b| {
        let mut key = 0u64;
        b.iter(|| {
            key = key.wrapping_add(1);
            sketch.estimate(black_box(key))
        });
    });
}

fn bench_cache_mixed_workload(c: &mut Criterion) {
    let cache: AliceCache<u64, u64> = AliceCache::new(10000);

    c.bench_function("cache_mixed_80read_20write", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i = i.wrapping_add(1);
            if i.is_multiple_of(5) {
                cache.put(black_box(i), black_box(i));
            } else {
                cache.get(black_box(&(i % 10000)));
            }
        });
    });
}

criterion_group!(
    benches,
    bench_cache_put,
    bench_cache_get_hit,
    bench_cache_get_miss,
    bench_jump_hash,
    bench_count_min_sketch,
    bench_cache_mixed_workload,
);
criterion_main!(benches);
