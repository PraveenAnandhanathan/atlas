//! Microbench: `LocalChunkStore` put/get vs. naive filesystem write/read.

use atlas_chunk::{ChunkStore, LocalChunkStore};
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::fs;
use std::io::{Read, Write};
use tempfile::TempDir;

fn bench_chunk_put(c: &mut Criterion) {
    let payload = vec![0xab_u8; 64 * 1024];
    let mut group = c.benchmark_group("chunk_put_64k");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    let dir = TempDir::new().unwrap();
    let store = LocalChunkStore::open(dir.path()).unwrap();
    group.bench_function("atlas_chunk", |b| {
        b.iter(|| {
            let h = store.put(black_box(&payload)).unwrap();
            black_box(h);
        });
    });

    let dir = TempDir::new().unwrap();
    let mut counter = 0u64;
    group.bench_function("naive_fs", |b| {
        b.iter(|| {
            let path = dir.path().join(format!("f{counter}"));
            counter += 1;
            let mut f = fs::File::create(&path).unwrap();
            f.write_all(black_box(&payload)).unwrap();
        });
    });

    group.finish();
}

fn bench_chunk_get(c: &mut Criterion) {
    let payload = vec![0xcd_u8; 64 * 1024];
    let mut group = c.benchmark_group("chunk_get_64k");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    let dir = TempDir::new().unwrap();
    let store = LocalChunkStore::open(dir.path()).unwrap();
    let h = store.put(&payload).unwrap();
    group.bench_function("atlas_chunk", |b| {
        b.iter(|| {
            let v = store.get(black_box(&h)).unwrap();
            black_box(v);
        });
    });

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("blob");
    fs::write(&path, &payload).unwrap();
    group.bench_function("naive_fs", |b| {
        b.iter(|| {
            let mut buf = Vec::with_capacity(payload.len());
            let mut f = fs::File::open(&path).unwrap();
            f.read_to_end(&mut buf).unwrap();
            black_box(buf);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_chunk_put, bench_chunk_get);
criterion_main!(benches);
