"""Provider interfaces keep model implementations out of the protocol layer."""

from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence


@dataclass(frozen=True, slots=True)
class CaptionResult:
    description: str
    tags: tuple[str, ...] = ()
    model: str | None = None


@dataclass(frozen=True, slots=True)
class OcrResult:
    text: str
    language: str | None = None
    model: str | None = None


@dataclass(frozen=True, slots=True)
class EmbeddingResult:
    vector: tuple[float, ...]
    model: str


class AiProvider(ABC):
    """Replaceable implementation for one local or remote AI backend."""

    @property
    @abstractmethod
    def name(self) -> str:
        """Stable provider identifier."""

    def caption(self, path: Path) -> CaptionResult:
        raise NotImplementedError(f"{self.name} does not support image captioning")

    def ocr(self, path: Path) -> OcrResult:
        raise NotImplementedError(f"{self.name} does not support OCR")

    def embed(self, texts: Sequence[str]) -> list[EmbeddingResult]:
        raise NotImplementedError(f"{self.name} does not support embeddings")
