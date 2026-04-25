# atlas-sdk (Python) — v0.1

Thin Python wrapper around the `atlasctl` binary. The surface is small
on purpose: enough for ML scripts to read/write/branch/commit while the
native bindings (PyO3 + gRPC) come together in Phase 2+.

```python
import atlas

store = atlas.Store("./.atlas-store")     # opens; init separately if needed
store.write("/data/sample.txt", b"hello")
print(store.read("/data/sample.txt"))     # b"hello"

with store.branch("experiment-1") as b:
    store.write("/data/sample.txt", b"world")
    b.commit("retrain on new sample")

for entry in store.list("/data"):
    print(entry.path, entry.hash, entry.size)
```

## Status

- Wraps `atlasctl` via `subprocess`.
- Replaced by a PyO3 binding in Phase 2 — the public API stays stable.
