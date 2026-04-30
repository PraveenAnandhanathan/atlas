//! End-to-end test: bring up an atlas-storage server, connect a
//! RemoteChunkStore + RemoteMetaStore, exercise the trait surface.

use atlas_chunk::ChunkStore;
use atlas_meta::MetaStore;
use atlas_net::{RemoteChunkStore, RemoteMetaStore};
use atlas_object::{Branch, BranchProtection, StoreConfig};
use atlas_storage::server::serve_with_addr;
use atlas_storage::ServerConfig;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chunk_and_meta_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let chunks_dir = dir.path().join("chunks");
    let meta_dir = dir.path().join("meta");
    let cfg = ServerConfig {
        bind: "127.0.0.1:0".into(),
        chunks_dir,
        meta_dir,
    };
    let (addr, _handle) = serve_with_addr(cfg).await.unwrap();
    let endpoint = format!("{addr}");

    // Move client work to a blocking task because the trait surface is sync.
    tokio::task::spawn_blocking(move || {
        let chunks = RemoteChunkStore::connect(endpoint.clone()).unwrap();
        let h = chunks.put(b"hello world").unwrap();
        assert_eq!(chunks.get(&h).unwrap(), b"hello world");
        assert!(chunks.has(&h).unwrap());
        chunks.verify(&h).unwrap();
        assert_eq!(chunks.size(&h).unwrap(), 11);
        assert_eq!(chunks.iter_hashes().count(), 1);

        let meta = RemoteMetaStore::connect(endpoint).unwrap();
        let cfg = StoreConfig::new();
        meta.put_config(&cfg).unwrap();
        let back = meta.get_config().unwrap().unwrap();
        assert_eq!(back.default_branch, "main");

        let b = Branch {
            name: "feature-x".into(),
            head: h,
            protection: BranchProtection::default(),
        };
        meta.put_branch(&b).unwrap();
        let names: Vec<_> = meta
            .list_branches()
            .unwrap()
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(names.contains(&"feature-x".to_string()));
    })
    .await
    .unwrap();
}
