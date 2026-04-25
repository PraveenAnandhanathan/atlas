"""ATLAS Embedder Service (T3.1).

Exposes a small HTTP API that converts text to dense embedding vectors
using sentence-transformers. Designed to run alongside the Rust storage
cluster — the Rust `EmbedderClient` in atlas-ingest calls this service.

Usage:
    pip install -e .
    atlas-embedder                        # default: 0.0.0.0:8765
    ATLAS_EMBEDDER_MODEL=all-MiniLM-L6-v2 atlas-embedder --port 8765

Environment variables:
    ATLAS_EMBEDDER_MODEL   — sentence-transformers model name
                             (default: "all-MiniLM-L6-v2")
    ATLAS_EMBEDDER_HOST    — bind host (default: 0.0.0.0)
    ATLAS_EMBEDDER_PORT    — bind port (default: 8765)
    ATLAS_EMBEDDER_DEVICE  — "cpu", "cuda", "mps" (default: "cpu")
"""

from __future__ import annotations

import os
import time
import logging
from typing import Optional

import uvicorn
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field
from sentence_transformers import SentenceTransformer

logger = logging.getLogger("atlas-embedder")

# ---------------------------------------------------------------------------
# Model registry (T3.7: model-version tagging)
# ---------------------------------------------------------------------------

DEFAULT_MODEL = os.getenv("ATLAS_EMBEDDER_MODEL", "all-MiniLM-L6-v2")
DEVICE = os.getenv("ATLAS_EMBEDDER_DEVICE", "cpu")

_registry: dict[str, SentenceTransformer] = {}
_current_model_name: str = DEFAULT_MODEL


def _load_model(name: str) -> SentenceTransformer:
    if name not in _registry:
        logger.info("Loading model %s on %s …", name, DEVICE)
        t0 = time.perf_counter()
        _registry[name] = SentenceTransformer(name, device=DEVICE)
        logger.info("Model %s loaded in %.1f s", name, time.perf_counter() - t0)
    return _registry[name]


def _current_model() -> SentenceTransformer:
    return _load_model(_current_model_name)


def _model_version() -> str:
    """Return a stable tag for the current model (name + dim)."""
    m = _current_model()
    dim = m.get_sentence_embedding_dimension()
    return f"{_current_model_name}@dim{dim}"


# ---------------------------------------------------------------------------
# FastAPI app
# ---------------------------------------------------------------------------

app = FastAPI(
    title="ATLAS Embedder",
    version="0.1.0",
    description="Text → dense embedding via sentence-transformers.",
)


class EmbedRequest(BaseModel):
    text: str = Field(..., description="Text to embed.")
    model: Optional[str] = Field(None, description="Override model name.")


class EmbedResponse(BaseModel):
    embedding: list[float]
    model_version: str
    dim: int


class BatchRequest(BaseModel):
    texts: list[str] = Field(..., description="Texts to embed.")
    model: Optional[str] = Field(None, description="Override model name.")


class BatchResponse(BaseModel):
    embeddings: list[list[float]]
    model_version: str
    dim: int


class HealthResponse(BaseModel):
    status: str
    model: str
    device: str


class ModelsResponse(BaseModel):
    current: str
    available: list[str]


class SwitchModelRequest(BaseModel):
    model: str


@app.on_event("startup")
def startup() -> None:
    _load_model(_current_model_name)
    logger.info("Embedder ready. Model: %s", _model_version())


@app.get("/health", response_model=HealthResponse)
def health() -> HealthResponse:
    return HealthResponse(
        status="ok",
        model=_model_version(),
        device=DEVICE,
    )


@app.get("/models", response_model=ModelsResponse)
def models() -> ModelsResponse:
    return ModelsResponse(
        current=_model_version(),
        available=list(_registry.keys()),
    )


@app.post("/models/switch")
def switch_model(req: SwitchModelRequest) -> dict:
    global _current_model_name
    _load_model(req.model)
    _current_model_name = req.model
    return {"status": "switched", "model_version": _model_version()}


@app.post("/embed", response_model=EmbedResponse)
def embed(req: EmbedRequest) -> EmbedResponse:
    if not req.text.strip():
        raise HTTPException(status_code=422, detail="text must be non-empty")
    model_name = req.model or _current_model_name
    model = _load_model(model_name)
    vec = model.encode(req.text, normalize_embeddings=True).tolist()
    return EmbedResponse(
        embedding=vec,
        model_version=_model_version(),
        dim=len(vec),
    )


@app.post("/embed_batch", response_model=BatchResponse)
def embed_batch(req: BatchRequest) -> BatchResponse:
    if not req.texts:
        raise HTTPException(status_code=422, detail="texts must be non-empty")
    model_name = req.model or _current_model_name
    model = _load_model(model_name)
    vecs = model.encode(req.texts, normalize_embeddings=True, show_progress_bar=False)
    return BatchResponse(
        embeddings=vecs.tolist(),
        model_version=_model_version(),
        dim=vecs.shape[1] if vecs.ndim == 2 else 0,
    )


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------

def run() -> None:
    import argparse

    p = argparse.ArgumentParser(description="ATLAS embedder service")
    p.add_argument("--host", default=os.getenv("ATLAS_EMBEDDER_HOST", "0.0.0.0"))
    p.add_argument("--port", type=int, default=int(os.getenv("ATLAS_EMBEDDER_PORT", "8765")))
    p.add_argument("--log-level", default="info")
    args = p.parse_args()

    logging.basicConfig(level=args.log_level.upper())
    uvicorn.run(app, host=args.host, port=args.port, log_level=args.log_level)


if __name__ == "__main__":
    run()
