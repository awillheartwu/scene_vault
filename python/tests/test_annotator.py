from __future__ import annotations

import unittest

from PIL import Image, ImageDraw

from scene_vault_ai.vision.annotator import ImageAnnotator
from scene_vault_ai.vision.config import AnnotationConfig
from scene_vault_ai.vision.types import FaceBox


class AnnotatorPositionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.image = Image.new("RGB", (400, 300), "black")
        self.draw = ImageDraw.Draw(self.image)
        self.face = FaceBox(x=100, y=100, width=60, height=40)

    def _text_metrics(self, annotator: ImageAnnotator) -> tuple[int, int]:
        box = self.draw.textbbox(
            (0, 0),
            "名字",
            font=annotator.font,
            stroke_width=annotator.config.stroke_width,
            anchor="lt",
        )
        return box[2] - box[0], box[3] - box[1]

    def test_custom_offset_centers_text_on_face_relative_point(self) -> None:
        config = AnnotationConfig(
            padding=10,
            font_size=24,
            face_text_position="custom",
            text_offset_x=0.5,
            text_offset_y=-0.5,
        )
        annotator = ImageAnnotator(config)
        text_width, text_height = self._text_metrics(annotator)

        left, top = annotator._text_position(
            self.draw, "名字", self.image.size, self.face
        )

        center_x = self.face.x + self.face.width * (0.5 + 0.5)
        center_y = self.face.y + self.face.height * (0.5 - 0.5)
        self.assertEqual(left, round(center_x - text_width / 2))
        self.assertEqual(top, round(center_y - text_height / 2))

    def test_custom_offset_out_of_bounds_falls_back_to_chain(self) -> None:
        config = AnnotationConfig(
            padding=10,
            font_size=24,
            face_text_position="custom",
            text_offset_x=3.0,
            text_offset_y=3.0,
        )
        annotator = ImageAnnotator(config)
        _, text_height = self._text_metrics(annotator)
        corner_face = FaceBox(x=300, y=240, width=60, height=40)

        left, top = annotator._text_position(
            self.draw, "名字", self.image.size, corner_face
        )

        # Custom point (510, 380) is off-image; the chain starts at "above".
        self.assertEqual(left, corner_face.x)
        self.assertEqual(top, corner_face.y - text_height - config.padding)

    def test_discrete_position_unchanged(self) -> None:
        config = AnnotationConfig(padding=10, font_size=24, face_text_position="right")
        annotator = ImageAnnotator(config)

        left, top = annotator._text_position(
            self.draw, "名字", self.image.size, self.face
        )

        self.assertEqual(left, self.face.x + self.face.width + config.padding)
        self.assertEqual(top, self.face.y)

    def test_default_expansion_zero_preserves_face_box(self) -> None:
        annotator = ImageAnnotator(AnnotationConfig())

        self.assertEqual(
            annotator._expanded_face_box(self.face),
            (self.face.x, self.face.y, self.face.width, self.face.height),
        )

    def test_face_box_expansion_shifts_preferred_positions(self) -> None:
        expansion = 24
        config = AnnotationConfig(
            padding=10,
            font_size=24,
            face_text_position="right",
            face_box_expansion=expansion,
        )
        annotator = ImageAnnotator(config)

        left, top = annotator._text_position(
            self.draw, "名字", self.image.size, self.face
        )

        self.assertEqual(
            left, self.face.x + self.face.width + expansion + config.padding
        )
        self.assertEqual(top, self.face.y - expansion)

    def test_custom_offset_scales_against_expanded_face_box(self) -> None:
        expansion = 20
        config = AnnotationConfig(
            padding=10,
            font_size=24,
            face_text_position="custom",
            text_offset_x=0.5,
            text_offset_y=-0.5,
            face_box_expansion=expansion,
        )
        annotator = ImageAnnotator(config)
        text_width, text_height = self._text_metrics(annotator)

        left, top = annotator._text_position(
            self.draw, "名字", self.image.size, self.face
        )

        ref_x = self.face.x - expansion
        ref_y = self.face.y - expansion
        ref_width = self.face.width + 2 * expansion
        ref_height = self.face.height + 2 * expansion
        center_x = ref_x + ref_width * (0.5 + 0.5)
        center_y = ref_y + ref_height * (0.5 - 0.5)
        self.assertEqual(left, round(center_x - text_width / 2))
        self.assertEqual(top, round(center_y - text_height / 2))


if __name__ == "__main__":
    unittest.main()
