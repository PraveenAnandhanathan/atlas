"""Subprocess wrapper around the atlasctl binary."""

from __future__ import annotations

import dataclasses
import os
import shutil
import subprocess
import sys
from contextlib import contextmanager
from pathlib import Path
from typing import Iterator, List, Optional, Sequence, Union

PathLike = Union[str, os.PathLike]


class AtlasError(RuntimeError):
    """Raised when atlasctl exits non-zero or output cannot be parsed."""


def locate_atlasctl() -> str:
    """Find the atlasctl binary on PATH or in common build locations.

    Search order:
      1. $ATLASCTL environment variable.
      2. shutil.which("atlasctl").
      3. ./target/release/atlasctl (workspace dev binary).
      4. ./target/debug/atlasctl.
    """
    env = os.environ.get("ATLASCTL")
    if env and Path(env).exists():
        return env
    found = shutil.which("atlasctl")
    if found:
        return found
    for candidate in ("target/release/atlasctl", "target/debug/atlasctl"):
        p = Path(candidate)
        if p.exists():
            return str(p.resolve())
        # Windows variant
        pw = Path(candidate + ".exe")
        if pw.exists():
            return str(pw.resolve())
    raise AtlasError(
        "atlasctl binary not found. Set $ATLASCTL or `cargo build --release -p atlasctl`."
    )


@dataclasses.dataclass
class Entry:
    """A file or directory entry in an ATLAS store."""

    path: str
    kind: str  # "file" | "dir" | "symlink" | "refspec"
    hash: str
    size: int


class Store:
    """A handle to a local ATLAS store.

    Phase 0/1 implementation: every method shells out to atlasctl. The
    public surface is what the Phase 2 PyO3 binding will keep stable.
    """

    def __init__(
        self,
        path: PathLike,
        *,
        atlasctl: Optional[str] = None,
        author_name: Optional[str] = None,
        author_email: Optional[str] = None,
    ) -> None:
        self.path = Path(path).resolve()
        self._bin = atlasctl or locate_atlasctl()
        self._author_name = author_name or os.environ.get("ATLAS_AUTHOR_NAME")
        self._author_email = author_email or os.environ.get("ATLAS_AUTHOR_EMAIL")

    # -- lifecycle -----------------------------------------------------

    @classmethod
    def init(
        cls,
        path: PathLike,
        *,
        atlasctl: Optional[str] = None,
    ) -> "Store":
        """Create a new store and return a handle to it."""
        store = cls(path, atlasctl=atlasctl)
        store._run(["init"])
        return store

    # -- file ops ------------------------------------------------------

    def write(self, atlas_path: str, data: bytes) -> Entry:
        """Write `data` to `atlas_path`. Creates intermediate dirs."""
        out = self._run(["put", atlas_path], stdin=data)
        return self.stat(atlas_path) if not out else self._parse_put(out, atlas_path)

    def read(self, atlas_path: str) -> bytes:
        """Read file contents."""
        return self._run(["cat", atlas_path], capture_bytes=True)

    def stat(self, atlas_path: str) -> Entry:
        out = self._run(["stat", atlas_path])
        return _parse_stat(out)

    def list(self, atlas_path: str = "/") -> List[Entry]:
        out = self._run(["ls", atlas_path])
        return _parse_ls(out, atlas_path)

    def delete(self, atlas_path: str, *, recursive: bool = False) -> None:
        args = ["rm", atlas_path]
        if recursive:
            args.append("--recursive")
        self._run(args)

    def rename(self, src: str, dst: str) -> None:
        self._run(["mv", src, dst])

    def mkdir(self, atlas_path: str) -> None:
        self._run(["mkdir", atlas_path])

    # -- versioning ----------------------------------------------------

    def commit(self, message: str) -> str:
        args = ["commit", "--message", message]
        if self._author_name:
            args.extend(["--author-name", self._author_name])
        if self._author_email:
            args.extend(["--author-email", self._author_email])
        out = self._run(args).strip()
        # "commit <hex>"
        return out.split()[-1] if out else ""

    def checkout(self, target: str) -> None:
        self._run(["checkout", target])

    def log(self, limit: int = 20) -> List[dict]:
        out = self._run(["log", "--limit", str(limit)])
        return _parse_log(out)

    @contextmanager
    def branch(self, name: str) -> Iterator["BranchContext"]:
        """Create or switch to `name`, yield a BranchContext."""
        existing = {b["name"] for b in self.branches()}
        if name not in existing:
            self._run(["branch", "create", name])
        previous = self._current_branch()
        self._run(["checkout", name])
        ctx = BranchContext(self, name)
        try:
            yield ctx
        finally:
            if previous and previous != name:
                self._run(["checkout", previous])

    def branches(self) -> List[dict]:
        out = self._run(["branch", "list"])
        rows = []
        for line in out.splitlines():
            line = line.strip()
            if not line:
                continue
            current = line.startswith("*")
            line = line.lstrip("* ").strip()
            parts = line.split()
            if len(parts) >= 2:
                rows.append({"name": parts[0], "head": parts[1], "current": current})
        return rows

    # -- internals -----------------------------------------------------

    def _current_branch(self) -> Optional[str]:
        for b in self.branches():
            if b["current"]:
                return b["name"]
        return None

    def _run(
        self,
        args: Sequence[str],
        *,
        stdin: Optional[bytes] = None,
        capture_bytes: bool = False,
    ) -> str | bytes:
        cmd = [self._bin, "--store", str(self.path), *args]
        try:
            result = subprocess.run(
                cmd,
                input=stdin,
                capture_output=True,
                check=False,
            )
        except FileNotFoundError as e:
            raise AtlasError(f"atlasctl not executable at {self._bin}: {e}") from e
        if result.returncode != 0:
            err = result.stderr.decode("utf-8", errors="replace").strip()
            raise AtlasError(f"atlasctl {' '.join(args)} failed: {err}")
        if capture_bytes:
            return result.stdout
        return result.stdout.decode("utf-8", errors="replace")

    def _parse_put(self, out: str, atlas_path: str) -> Entry:
        # "<short-hash> <path> <size> bytes"
        parts = out.strip().split()
        if len(parts) >= 3:
            try:
                size = int(parts[2])
            except ValueError:
                size = 0
            # Re-stat to get the full hash; cheap.
            return self.stat(atlas_path)
        return self.stat(atlas_path)


class BranchContext:
    """Returned by `Store.branch()`. Use `.commit(msg)` to seal changes."""

    def __init__(self, store: Store, name: str) -> None:
        self.store = store
        self.name = name

    def commit(self, message: str) -> str:
        return self.store.commit(message)


# -- output parsers ----------------------------------------------------


def _parse_stat(out: str) -> Entry:
    fields = {}
    for line in out.splitlines():
        if ":" not in line:
            continue
        k, v = line.split(":", 1)
        fields[k.strip()] = v.strip()
    try:
        return Entry(
            path=fields["path"],
            kind=fields["kind"],
            hash=fields["hash"],
            size=int(fields.get("size", "0")),
        )
    except KeyError as e:
        raise AtlasError(f"could not parse stat output: {out!r}") from e


def _parse_ls(out: str, parent: str) -> List[Entry]:
    rows: List[Entry] = []
    for line in out.splitlines():
        line = line.rstrip()
        if not line:
            continue
        # "{:>10}  {short}  {name}{mark}" — split on whitespace.
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        try:
            size = int(parts[0])
        except ValueError:
            continue
        short_hash = parts[1]
        name = parts[2]
        kind = "file"
        if name.endswith("/"):
            kind = "dir"
            name = name[:-1]
        elif name.endswith("@"):
            kind = "symlink"
            name = name[:-1]
        path = parent.rstrip("/") + "/" + name if parent != "/" else "/" + name
        rows.append(Entry(path=path, kind=kind, hash=short_hash, size=size))
    return rows


def _parse_log(out: str) -> List[dict]:
    commits: List[dict] = []
    current: dict = {}
    for line in out.splitlines():
        line = line.rstrip()
        if line.startswith("commit "):
            if current:
                commits.append(current)
            current = {"hash": line.split(None, 1)[1].strip(), "message": ""}
        elif line.startswith("Author:"):
            current["author"] = line.split(":", 1)[1].strip()
        elif line.startswith("Date:"):
            current["date"] = line.split(":", 1)[1].strip()
        elif line.startswith("    "):
            current["message"] = (current.get("message", "") + line[4:] + "\n").rstrip("\n")
    if current:
        commits.append(current)
    return commits


if __name__ == "__main__":
    print(f"atlas-sdk v0.1, atlasctl: {locate_atlasctl()}", file=sys.stderr)
