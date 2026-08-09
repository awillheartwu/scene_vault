from __future__ import annotations

from typing import Any

from .config import CropConfig
from .types import FaceBox


class AvatarCropper:
    def __init__(self, config: CropConfig) -> None:
        self.config = config

    def crop(self, image: Any, face_box: FaceBox) -> Any:
        image_width, image_height = image.size
        center_x = face_box.x + face_box.width / 2
        crop_width = max(
            face_box.width * self.config.scale_x,
            self.config.min_size,
        )
        crop_height = max(
            face_box.height
            * (self.config.scale_top + self.config.scale_bottom),
            self.config.min_size,
        )

        aspect_width, aspect_height = self.config.aspect_ratio
        target_ratio = aspect_width / aspect_height
        if crop_width / crop_height < target_ratio:
            crop_width = crop_height * target_ratio
        else:
            crop_height = crop_width / target_ratio

        left = int(round(center_x - crop_width / 2))
        top = int(
            round(
                face_box.y
                - face_box.height * (self.config.scale_top - 0.5)
            )
        )
        right = int(round(left + crop_width))
        bottom = int(round(top + crop_height))

        if left < 0:
            right -= left
            left = 0
        if top < 0:
            bottom -= top
            top = 0
        if right > image_width:
            left -= right - image_width
            right = image_width
        if bottom > image_height:
            top -= bottom - image_height
            bottom = image_height

        return image.crop(
            (
                max(0, left),
                max(0, top),
                min(image_width, right),
                min(image_height, bottom),
            )
        )
