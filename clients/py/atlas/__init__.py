"""atlas — Python SDK v0.1.

A thin subprocess-based client for the local atlasctl binary. The
public surface (Store, Entry, BranchContext) is the API the Phase 2
PyO3 binding will preserve.
"""

from .client import BranchContext, Entry, Store, AtlasError, locate_atlasctl

__all__ = [
    "Store",
    "Entry",
    "BranchContext",
    "AtlasError",
    "locate_atlasctl",
]

__version__ = "0.1.0"
