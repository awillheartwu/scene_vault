from __future__ import annotations

import math
from dataclasses import asdict, dataclass


@dataclass(frozen=True, slots=True)
class FaceBox:
    x: int
    y: int
    width: int
    height: int
    confidence: float = 0.0
    # YuNet's five facial landmarks: (left eye, right eye, nose tip, left
    # mouth corner, right mouth corner). Used by alignCrop/alignment; passing
    # them makes the aligned crop deterministic across processes (a bare
    # bounding box is not).
    landmarks: tuple[tuple[float, float], ...] | None = None

    def __post_init__(self) -> None:
        if self.x < 0 or self.y < 0:
            raise ValueError("face coordinates must not be negative")
        if self.width <= 0 or self.height <= 0:
            raise ValueError("face dimensions must be positive")
        if not math.isfinite(self.confidence) or not 0.0 <= self.confidence <= 1.0:
            raise ValueError("face confidence must be between 0 and 1")
        if self.landmarks is not None and len(self.landmarks) != 5:
            raise ValueError("face landmarks must contain exactly 5 points")

    @property
    def area(self) -> int:
        return self.width * self.height

    @property
    def center(self) -> tuple[float, float]:
        return (self.x + self.width / 2, self.y + self.height / 2)

    def to_dict(self) -> dict[str, int | float]:
        return asdict(self)
