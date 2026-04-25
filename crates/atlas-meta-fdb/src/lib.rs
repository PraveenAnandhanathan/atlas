//! FoundationDB-backed [`MetaStore`](atlas_meta::MetaStore).
//!
//! This crate is feature-gated on `fdb`. Without the feature, the crate
//! still compiles (so the workspace builds on machines without
//! libfdb_c) but exposes only a stub. Enable `--features fdb` together
//! with the FoundationDB client library to use the real backend.
//!
//! Why FDB?
//! - Strict serializable transactions are exactly the model `Transaction`
//!   in `atlas-meta` was designed against.
//! - Horizontally scalable beyond a single sled instance, which is the
//!   bottleneck once a cluster has more than ~1 PB of metadata.
//!
//! The schema is identical to the sled backend — same key layout from
//! `atlas_meta::keys` — so a sled → fdb migration is a key/value copy.

#[cfg(feature = "fdb")]
pub mod imp {
    use atlas_core::{Error, Result};
    use atlas_meta::{MetaStore, Transaction, TxOpExternal};
    use foundationdb::{tuple::Subspace, Database};

    pub struct FdbMetaStore {
        db: Database,
        subspace: Subspace,
    }

    impl FdbMetaStore {
        /// Open a FoundationDB cluster file. Pass `None` to use the
        /// default file path (`/etc/foundationdb/fdb.cluster`).
        pub fn open(cluster_file: Option<&str>, namespace: &str) -> Result<Self> {
            let db = Database::new(cluster_file)
                .map_err(|e| Error::Backend(format!("fdb open: {e}")))?;
            Ok(Self {
                db,
                subspace: Subspace::all().subspace(&namespace),
            })
        }

        fn key(&self, k: &str) -> Vec<u8> {
            self.subspace.subspace(&k).bytes().to_vec()
        }
    }

    impl MetaStore for FdbMetaStore {
        fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>> {
            let k = self.key(key);
            let res = futures::executor::block_on(async {
                let trx = self.db.create_trx()?;
                let v = trx.get(&k, false).await?;
                Ok::<_, foundationdb::FdbError>(v.map(|s| s.to_vec()))
            })
            .map_err(|e| Error::Backend(format!("fdb get: {e}")))?;
            Ok(res)
        }

        fn put_raw(&self, key: &str, value: &[u8]) -> Result<()> {
            let k = self.key(key);
            futures::executor::block_on(async {
                let trx = self.db.create_trx()?;
                trx.set(&k, value);
                trx.commit().await.map_err(|e| e.into())
            })
            .map_err(|e: foundationdb::FdbError| Error::Backend(format!("fdb put: {e}")))?;
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<()> {
            let k = self.key(key);
            futures::executor::block_on(async {
                let trx = self.db.create_trx()?;
                trx.clear(&k);
                trx.commit().await.map_err(|e| e.into())
            })
            .map_err(|e: foundationdb::FdbError| Error::Backend(format!("fdb delete: {e}")))?;
            Ok(())
        }

        fn scan_prefix(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
            let pk = self.key(prefix);
            let mut end = pk.clone();
            end.push(0xff);
            let res = futures::executor::block_on(async {
                let trx = self.db.create_trx()?;
                let opt = foundationdb::RangeOption::from((pk.as_slice(), end.as_slice()));
                let kvs = trx.get_range(&opt, 1, false).await?;
                Ok::<_, foundationdb::FdbError>(
                    kvs.iter()
                        .map(|kv| {
                            (
                                String::from_utf8_lossy(kv.key()).into_owned(),
                                kv.value().to_vec(),
                            )
                        })
                        .collect(),
                )
            })
            .map_err(|e| Error::Backend(format!("fdb scan: {e}")))?;
            Ok(res)
        }

        fn apply(&self, tx: Transaction) -> Result<()> {
            let ops = tx.into_ops();
            futures::executor::block_on(async {
                let trx = self.db.create_trx()?;
                for op in &ops {
                    match op {
                        TxOpExternal::Put { key, value } => {
                            trx.set(&self.key(key), value);
                        }
                        TxOpExternal::Delete { key } => {
                            trx.clear(&self.key(key));
                        }
                    }
                }
                trx.commit().await.map_err(|e| e.into())
            })
            .map_err(|e: foundationdb::FdbError| Error::Backend(format!("fdb commit: {e}")))?;
            Ok(())
        }
    }
}

#[cfg(not(feature = "fdb"))]
pub mod imp {
    //! Stub when the `fdb` feature is off. Constructing the store
    //! returns a clear error so callers know to enable the feature.
    use atlas_core::{Error, Result};

    pub struct FdbMetaStore;

    impl FdbMetaStore {
        pub fn open(_cluster_file: Option<&str>, _namespace: &str) -> Result<Self> {
            Err(Error::Backend(
                "atlas-meta-fdb built without the `fdb` feature".into(),
            ))
        }
    }
}

pub use imp::FdbMetaStore;
