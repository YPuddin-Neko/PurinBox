import json
import io
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace
from pathlib import Path

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import tagger_inference as tagger
import gpu_diagnostics


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

    def test_every_artist_has_a_prefix_and_schema_order_is_stable(self):
        selected = [("first", "artist"), ("@second", "artist")]
        full = tagger._build_structured_json(selected)
        simple = tagger._build_simplified_json(selected)
        self.assertEqual(full["fixed"]["artist"], "@first, @second")
        self.assertEqual(simple["artist"], "@first, @second")
        self.assertEqual(list(full), ["fixed", "character", "from_path", "ai_output"])
        self.assertEqual(list(full["character"]), ["name", "variant"])
        self.assertEqual(list(simple), ["quality", "series", "artist", "character", "count",
                                       "appearance", "tags", "environment", "nl"])

    def test_input_layout_handles_four_channels_and_dynamic_shapes_consistently(self):
        for shape, expected in [([1, 4, 32, 32], ("NCHW", 32)),
                                ([1, 32, 32, 4], ("NHWC", 32)),
                                (["N", 3, "H", 64], ("NCHW", 64)),
                                ([1, "H", "W", 3], ("NHWC", 448)),
                                ([1, 2], ("NHWC", 448))]:
            session = SimpleNamespace(get_inputs=lambda: [SimpleNamespace(shape=shape)])
            with self.subTest(shape=shape):
                self.assertEqual(tagger._input_layout(shape), expected)
                self.assertEqual(tagger.detect_model_format(session), expected)

    def test_provider_fallback_logs_only_the_provider_name(self):
        with patch.object(tagger, "log") as log:
            tagger._log_gpu_fallback(("CUDAExecutionProvider", {"private_option": "value"}), RuntimeError("failed"))
        self.assertEqual(log.call_args_list[0].args[0], "⚠ CUDAExecutionProvider 加载失败")

    def test_write_outputs_preserves_unreadable_existing_files(self):
        image = self.root / "image.png"
        tags = [{"name": "solo", "category": "general"}]
        for extension, original in [("json", b"{broken"), ("txt", b"\xff")]:
            path = image.with_suffix("." + extension)
            path.write_bytes(original)
            with patch.object(tagger, "log"):
                result = tagger._write_outputs(str(image), [.9], {
                    "output_format": extension, "existing_tags_action": "append",
                }, tags, {})
            self.assertTrue(result["skipped"])
            self.assertEqual(path.read_bytes(), original)

    def test_write_outputs_txt_merge_and_trigger_order(self):
        image = self.root / "image.png"
        path = image.with_suffix(".txt")
        path.write_text("existing, solo", encoding="utf-8")
        tags = [{"name": name, "category": "general"} for name in ["solo", "new"]]
        tagger._write_outputs(str(image), [.9, .8], {
            "existing_tags_action": "append", "append_tags": "trigger, solo",
            "append_position": "prepend",
        }, tags, {})
        self.assertEqual(path.read_text(), "trigger, solo, existing, new")


class ProtocolTests(unittest.TestCase):
    def run_protocol(self, commands, session, module=tagger, tags=None, provider_calls=None):
        messages = []
        tags = tags or [{"name": "solo", "category": "general"}]

        def providers(*args, **kwargs):
            if provider_calls is not None:
                provider_calls.append(kwargs)
            return ["CPUExecutionProvider"]

        with patch.dict(sys.modules, {"onnxruntime": SimpleNamespace()}), \
             patch.object(sys, "argv", ["tagger_inference.py"]), \
             patch.multiple(module, bootstrap=lambda: None,
                            utf8_stdin=lambda: io.StringIO("\n".join(map(json.dumps, commands))),
                            load_tags=lambda _: tags,
                            emit=messages.append,
                            result=lambda **fields: messages.append({"type": "result", **fields}),
                            error=lambda message, **fields: messages.append({"type": "error", "message": message, **fields}),
                            log=lambda message: messages.append({"type": "log", "message": message})), \
             patch.multiple(gpu_diagnostics,
                            quiet_session_options=lambda _: None,
                            resolve_ort_providers=providers,
                            create_session_with_cpu_fallback=lambda *a: session):
            module.main()
        return messages

    def test_cuda_policy_comes_from_init_flag_not_preprocessing_name(self):
        session = SimpleNamespace(
            get_inputs=lambda: [SimpleNamespace(name="input", shape=[1, 3, 32, 32])],
            get_outputs=lambda: [SimpleNamespace(name="output")],
        )
        calls = []
        self.run_protocol([
            {"cmd": "init", "model_path": "mock", "tags_path": "mock",
             "preprocess_mode": "auto", "conservative_cuda": True},
            {"cmd": "init", "model_path": "mock", "tags_path": "mock",
             "preprocess_mode": "pixai_v1", "conservative_cuda": False},
        ], session, provider_calls=calls)
        self.assertEqual(calls[0]["cuda_options"], {
            "cudnn_conv_algo_search": "HEURISTIC", "arena_extend_strategy": "kSameAsRequested",
            "do_copy_in_default_stream": "1",
        })
        self.assertIsNone(calls[1]["cuda_options"])

    def test_single_image_inference_failure_is_not_retried(self):
        calls = []

        def fail(*args):
            calls.append(args)
            raise RuntimeError("inference failed")

        session = SimpleNamespace(
            get_inputs=lambda: [SimpleNamespace(name="input", shape=[1, 2, 2, 3])],
            get_outputs=lambda: [SimpleNamespace(name="output")], run=fail,
        )
        with patch.object(tagger, "preprocess_image", return_value=np.zeros((1, 2, 2, 3))):
            messages = self.run_protocol([
                {"cmd": "init", "model_path": "mock", "tags_path": "mock"},
                {"cmd": "tag_batch", "images": [{"image_path": "mock.png"}]},
            ], session)
        self.assertEqual(len(calls), 1)
        errors = [m for m in messages if m["type"] == "error"]
        self.assertEqual(len(errors), 1)
        self.assertEqual(errors[0]["image_path"], "mock.png")

    def test_batch_failure_falls_back_to_each_image(self):
        calls = []

        def infer(_, inputs):
            count = len(inputs["input"])
            calls.append(count)
            if count > 1:
                raise RuntimeError("batch unsupported")
            return [np.array([[.9]], dtype=np.float32)]

        session = SimpleNamespace(
            get_inputs=lambda: [SimpleNamespace(name="input", shape=["N", 2, 2, 3])],
            get_outputs=lambda: [SimpleNamespace(name="output")], run=infer,
        )
        with tempfile.TemporaryDirectory() as root, \
             patch.object(tagger, "preprocess_image", return_value=np.zeros((1, 2, 2, 3))):
            paths = [str(Path(root) / f"{i}.png") for i in range(2)]
            messages = self.run_protocol([
                {"cmd": "init", "model_path": "mock", "tags_path": "mock"},
                {"cmd": "tag_batch", "images": [{"image_path": p} for p in paths]},
            ], session)
            self.assertEqual(calls, [2, 1, 1])
            self.assertEqual([m["image_path"] for m in messages if m["type"] == "result"], paths)
            for path in paths:
                self.assertEqual(Path(path).with_suffix(".txt").read_text(), "solo")


if __name__ == "__main__":
    unittest.main()
