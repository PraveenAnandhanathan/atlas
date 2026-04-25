//! Macrobench: full ATLAS write/read round-trip including manifest
//! creation and metadata writes — the realistic user-facing path.

use atlas_fs::Fs;
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use tempfile::TempDir;

fn bench_fs_write(c: &mut Criterion) {
    let payload = vec![0x42_u8; 256 * 1024];
    let mut group = c.benchmark_group("fs_write_256k");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    let dir = TempDir::new().unwrap();
    let fs = Fs::init(dir.path()).unwrap();
    let mut counter = 0u64;
    group.bench_function("atlas_fs", |b| {
        b.iter(|| {
            let path = format!("/f{counter}");
            counter += 1;
            fs.write(black_box(&path), black_box(&payload)).unwrap();
        });
    });

    group.finish();
}

fn bench_fs_read(c: &mut Criterion) {
    let payload = vec![0x33_u8; 256 * 1024];
    let mut group = c.benchmark_group("fs_read_256k");
    group.throughput(Throughput::Bytes(payload.len() as u64));

    let dir = TempDir::new().unwrap();
    let fs = Fs::init(dir.path()).unwrap();
    fs.write("/probe", &payload).unwrap();
    group.bench_function("atlas_fs", |b| {
        b.iter(|| {
            let v = fs.read(black_box("/probe")).unwrap();
            black_box(v);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_fs_write, bench_fs_read);
criterion_main!(benches);
