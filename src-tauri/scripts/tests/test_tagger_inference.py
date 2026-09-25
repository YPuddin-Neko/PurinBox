import json
import sys
import tempfile
import unittest
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import tagger_inference as tagger


PIXAI_THRESHOLDS = {
    "general": 0.17, "character": 0.27, "copyright": 0.24,
    "style": 0.15, "meta": 0.17, "rating": 0.41,
}


class PixaiTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def test_resize_keeps_aspect_ratio_and_normalizes_black_padding(self):
        path = self.root / "red.png"
        Image.new("RGB", (2, 1), (255, 0, 0)).save(path)
        data = tagger.preprocess_image(path, 4, "NCHW", "pixai_v1")
        self.assertEqual(data.shape, (1, 3, 4, 4))
        self.assertEqual(data.dtype, np.float32)
        np.testing.assert_array_equal(data[0, :, [0, 3], :], -1)
        np.testing.assert_array_equal(data[0, 0, 1:3, :], 1)
        np.testing.assert_array_equal(data[0, 1:, :, :], -1)

    def test_transparent_pixels_use_white_and_padding_stays_black(self):
        path = self.root / "alpha.png"
        Image.new("RGBA", (1, 2), (255, 0, 0, 0)).save(path)
        data = tagger.preprocess_image(path, 4, "NCHW", "pixai_v1")
        np.testing.assert_array_equal(data[0, :, :, 1:3], 1)
        np.testing.assert_array_equal(data[0, :, :, [0, 3]], -1)

    def test_extreme_aspect_ratio_keeps_at_least_one_pixel(self):
        path = self.root / "thin.png"
        Image.new("RGB", (1, 2000), (255, 255, 255)).save(path)
        self.assertEqual(tagger.preprocess_image(path, 4, "NCHW", "pixai_v1").shape, (1, 3, 4, 4))

    def vocabulary(self):
        return {
            "num_classes": 3,
            "categories": [
                {"name": "style", "offset": 2, "count": 1, "tags": ["watercolor"]},
                {"name": "general", "offset": 0, "count": 2, "tags": ["1girl", "solo"]},
            ],
        }

    def test_grouped_vocabulary_uses_offsets(self):
        path = self.root / "tags.json"
        path.write_text(json.dumps(self.vocabulary()))
        tags = tagger.load_tags_json(path)
        self.assertEqual([tag["name"] for tag in tags], ["1girl", "solo", "watercolor"])
        self.assertEqual(tags[-1]["category"], "style")

    def test_grouped_vocabulary_rejects_bad_offsets_counts_and_total(self):
        for key, value in [("offset", 1), ("count", 2)]:
            data = self.vocabulary()
            data["categories"][0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                tagger.load_grouped_tags_json(data)
        data = self.vocabulary()
        data["num_classes"] = 4
        with self.assertRaises(ValueError):
            tagger.load_grouped_tags_json(data)

    def test_all_six_thresholds_and_rating_is_not_argmax(self):
        tags, probs = [], []
        for category, threshold in PIXAI_THRESHOLDS.items():
            for suffix, probability in [("below", threshold - .01), ("above", threshold + .01)]:
                tags.append({"name": f"{category}-{suffix}", "category": category})
                probs.append(probability)
        _, names = tagger.select_tags(probs, tags, {"enabled_categories": list(PIXAI_THRESHOLDS)}, PIXAI_THRESHOLDS)
        self.assertEqual(set(names), {f"{category}-above" for category in PIXAI_THRESHOLDS})
        rating = [{"name": "rating:s", "category": "rating"}, {"name": "rating:g", "category": "rating"}]
        _, names = tagger.select_tags([.3, .2], rating, {"enabled_categories": ["rating"]}, PIXAI_THRESHOLDS)
        self.assertEqual(names, [])
        _, names = tagger.select_tags([.5, .6], rating, {"enabled_categories": ["rating"]}, PIXAI_THRESHOLDS)
        self.assertEqual(names, ["rating:g", "rating:s"])

    def test_user_thresholds_override_general_and_character_only(self):
        tags = [{"name": cat, "category": cat} for cat in PIXAI_THRESHOLDS]
        options = {"enabled_categories": list(PIXAI_THRESHOLDS), "general_threshold": .8, "character_threshold": .9}
        _, names = tagger.select_tags([.5] * 6, tags, options, PIXAI_THRESHOLDS)
        self.assertEqual(set(names), {"copyright", "style", "meta", "rating"})

    def test_category_toggles_filter_style(self):
        tags = [{"name": "watercolor", "category": "style"}]
        self.assertEqual(tagger.select_tags([.9], tags, {}, PIXAI_THRESHOLDS)[1], [])
        self.assertEqual(tagger.select_tags([.9], tags, {"enabled_categories": ["style"]}, PIXAI_THRESHOLDS)[1], ["watercolor"])

    def test_legacy_rating_quality_thresholds_and_frequency_sort(self):
        categories = ["rating", "rating", "quality", "quality", "general", "character", "copyright", "artist", "meta"]
        tags = [{"name": str(i), "category": cat, "count": i} for i, cat in enumerate(categories)]
        options = {"enabled_categories": categories, "sort_by": "frequency"}
        _, names = tagger.select_tags([.1, .2, .3, .2, .4, .9, .8, .9, .4], tags, options, {})
        self.assertEqual(names, ["8", "7", "5", "4", "2", "1"])

    def test_escaping_exclusion_and_kaomoji(self):
        tags = [{"name": name, "category": "general"} for name in ["o_o", "long_hair", "name_(series)"]]
        options = {"exclude_tags": "long_hair", "escape_parentheses": True}
        _, names = tagger.select_tags([.9] * 3, tags, options, {})
        self.assertEqual(names, ["o_o", r"name \(series\)"])

    def test_output_length_mismatch_is_rejected(self):
        with self.assertRaises(ValueError):
            tagger.select_tags([.9], [], {}, PIXAI_THRESHOLDS)

    def test_style_is_preserved_as_tags_in_both_json_formats(self):
        selected = [("blue hair", "style", .9, 0)]
        full = tagger._build_structured_json(selected)
        simple = tagger._build_simplified_json(selected)
        self.assertEqual(full["ai_output"]["tags"], ["blue hair"])
        self.assertEqual(simple["tags"], ["blue hair"])
        self.assertEqual(simple["artist"], "")
        self.assertEqual(simple["appearance"], [])
        for data in (full, simple):
            self.assertIn("blue hair", tagger._flatten_json_tags(data))


if __name__ == "__main__":
    unittest.main()
