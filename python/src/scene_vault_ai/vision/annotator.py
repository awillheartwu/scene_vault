from __future__ import annotations

import os
from pathlib import Path
from typing import Any

from ..errors import ResourceNotFoundError
from .config import AnnotationConfig
from .types import FaceBox


class ImageAnnotator:
    def __init__(self, config: AnnotationConfig) -> None:
        self.config = config
        self.font = _load_font(config)

    def annotate(self, image: Any, name: str, face_box: FaceBox | None) -> Any:
        from PIL import ImageDraw

        annotated = image.copy()
        draw = ImageDraw.Draw(annotated)
        left, top = self._text_position(draw, name, annotated.size, face_box)
        draw.text(
            (left, top),
            name,
            font=self.font,
            fill=self.config.text_color,
            stroke_width=self.config.stroke_width,
            stroke_fill=self.config.stroke_color,
            anchor="lt",
        )
        return annotated

    def _text_position(
        self,
        draw: Any,
        name: str,
        image_size: tuple[int, int],
        face_box: FaceBox | None,
    ) -> tuple[int, int]:
        padding = self.config.padding
        image_width, image_height = image_size
        text_box = draw.textbbox(
            (0, 0),
            name,
            font=self.font,
            stroke_width=self.config.stroke_width,
            anchor="lt",
        )
        text_width = text_box[2] - text_box[0]
        text_height = text_box[3] - text_box[1]

        if face_box is not None:
            preferred = {
                "above": (face_box.x, face_box.y - text_height - padding),
                "right": (
                    face_box.x + face_box.width + padding,
                    face_box.y,
                ),
                "below": (
                    face_box.x,
                    face_box.y + face_box.height + padding,
                ),
                "left": (face_box.x - text_width - padding, face_box.y),
            }
            if (
                self.config.face_text_position == "custom"
                and self.config.text_offset_x is not None
                and self.config.text_offset_y is not None
            ):
                # Place the text center at an arbitrary point relative to the
                # face box; offsets are multiples of the box width/height.
                center_x = (
                    face_box.x
                    + face_box.width * (0.5 + self.config.text_offset_x)
                )
                center_y = (
                    face_box.y
                    + face_box.height * (0.5 + self.config.text_offset_y)
                )
                first = (
                    int(round(center_x - text_width / 2)),
                    int(round(center_y - text_height / 2)),
                )
                remaining = [
                    preferred[direction]
                    for direction in ("above", "right", "below", "left")
                ]
            else:
                first = preferred.get(
                    self.config.face_text_position, preferred["above"]
                )
                remaining = [
                    preferred[direction]
                    for direction in ("above", "right", "below", "left")
                    if direction != self.config.face_text_position
                ]
            candidates = [first, *remaining]
            for left, top in candidates:
                if (
                    left >= padding
                    and top >= padding
                    and left + text_width <= image_width - padding
                    and top + text_height <= image_height - padding
                ):
                    return left, top

        positions = {
            "top_right": (image_width - text_width - padding, padding),
            "bottom_left": (padding, image_height - text_height - padding),
            "bottom_right": (
                image_width - text_width - padding,
                image_height - text_height - padding,
            ),
        }
        left, top = positions.get(self.config.fallback_position, (padding, padding))
        return max(padding, left), max(padding, top)


def _load_font(config: AnnotationConfig) -> Any:
    from PIL import ImageFont

    if config.font_path is not None:
        if not config.font_path.is_file():
            raise ResourceNotFoundError(
                "annotation font was not found",
                details={
                    "resource": "annotation_font",
                    "path": str(config.font_path),
                },
            )
        try:
            return ImageFont.truetype(
                str(config.font_path),
                size=config.font_size,
            )
        except OSError as error:
            raise ResourceNotFoundError(
                "annotation font could not be loaded",
                details={
                    "resource": "annotation_font",
                    "path": str(config.font_path),
                },
            ) from error

    for candidate in _system_font_candidates():
        if candidate.is_file():
            try:
                return ImageFont.truetype(str(candidate), size=config.font_size)
            except OSError:
                continue

    try:
        return ImageFont.load_default(size=config.font_size)
    except TypeError:  # Pillow before 10.1 did not expose the size argument.
        return ImageFont.load_default()


def _system_font_candidates() -> tuple[Path, ...]:
    windows_directory = os.environ.get("WINDIR") or os.environ.get("SystemRoot")
    windows_fonts = (
        Path(windows_directory) / "Fonts" if windows_directory else None
    )
    candidates = [
        windows_fonts / "msyh.ttc" if windows_fonts else None,
        windows_fonts / "msyhbd.ttc" if windows_fonts else None,
        windows_fonts / "simhei.ttf" if windows_fonts else None,
        windows_fonts / "arial.ttf" if windows_fonts else None,
        Path("/System/Library/Fonts/PingFang.ttc"),
        Path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        Path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ]
    return tuple(candidate for candidate in candidates if candidate is not None)
