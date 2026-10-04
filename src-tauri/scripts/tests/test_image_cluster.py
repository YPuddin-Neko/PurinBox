import json
import sys
import tempfile
import unittest
import warnings
from pathlib import Path
from unittest import mock

import numpy as np
from PIL import Image

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import image_cluster
from imagefixtures import png_bytes, stub_module


class ClusterPcaTests(unittest.TestCase):
    def test_pca_caps_dimension_by_samples_and_features(self):
        with mock.patch.object(image_cluster, 'log'):
            for n in (1, 2, 5, 49, 60):
                for width in (1, 8, 128):
                    with self.subTest(samples=n, width=width):
                        values = np.random.default_rng(42).normal(size=(n, width))
                        reduced = image_cluster._pca_reduce(values)
                        expected = min(50, width, n - 1) if n > 1 else width
                        self.assertEqual(reduced.shape, (n, expected))
                        self.assertTrue(np.isfinite(reduced).all())

    def test_hdbscan_without_umap_accepts_five_samples(self):
        features = np.random.default_rng(42).normal(size=(5, 100))
        with stub_module('umap', None), mock.patch.object(image_cluster, 'log'), \
                warnings.catch_warnings():
            warnings.simplefilter('ignore', FutureWarning)
            labels = image_cluster.cluster_hdbscan(features, min_cluster_size=2)
        self.assertEqual(labels.shape, (5,))


class ClusterLabelTests(unittest.TestCase):
    def labels(self, n_samples, algorithm, n_clusters=8, min_cluster_size=5):
        features = np.random.default_rng(0).normal(size=(n_samples, 16))
        with stub_module('umap', None), \
                mock.patch.object(image_cluster, 'log') as log, warnings.catch_warnings():
            warnings.simplefilter('ignore', FutureWarning)
            labels = image_cluster.cluster_labels(features, algorithm, n_clusters, min_cluster_size)
        return labels, ' '.join(str(c.args[0]) for c in log.call_args_list)

    def test_hdbscan_with_fewer_samples_than_min_cluster_size_falls_back_to_kmeans(self):
        for n_samples in (2, 3, 4):
            with self.subTest(samples=n_samples):
                with mock.patch.object(image_cluster, 'cluster_hdbscan') as hdbscan:
                    labels, logged = self.labels(n_samples, 'hdbscan', min_cluster_size=5)
                hdbscan.assert_not_called()
                self.assertEqual(len(labels), n_samples)
                self.assertTrue(all(label >= 0 for label in labels))
                self.assertIn('少于最小簇大小 5', logged)

    def test_hdbscan_without_any_cluster_falls_back_to_kmeans(self):
        with mock.patch.object(image_cluster, 'cluster_hdbscan', return_value=np.full(6, -1)):
            labels, logged = self.labels(6, 'hdbscan', min_cluster_size=3)
        self.assertEqual(len(set(labels)), 2)
        self.assertIn('未找到有效分组', logged)

    def test_hdbscan_runs_when_samples_suffice(self):
        labels, logged = self.labels(5, 'hdbscan', min_cluster_size=5)
        self.assertEqual(len(labels), 5)
        self.assertNotIn('少于最小簇大小', logged)

    def test_kmeans_caps_cluster_count_by_samples(self):
        labels, _ = self.labels(3, 'kmeans', n_clusters=8)
        self.assertEqual(sorted(set(labels)), [0, 1, 2])


class ManifestTests(unittest.TestCase):
    def test_reads_paths_with_relative_dirs(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / 'files.json'
            manifest.write_text(json.dumps([['/data/a.png', ''], ['/data/sub/b.png', 'sub']]), encoding='utf-8')
            self.assertEqual(image_cluster.read_manifest(manifest), [('/data/a.png', ''), ('/data/sub/b.png', 'sub')])


class MainTests(unittest.TestCase):
    """主流程：特征提取用替身，检查分组目录、子目录保留和 done 消息"""

    def test_copies_into_cluster_dirs_keeping_relative_dirs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'in/sub').mkdir(parents=True)
            files = []
            for i, rel in enumerate(['', '', 'sub']):
                path = root / 'in' / rel / f'{i}.png'
                path.write_bytes(png_bytes(np.full((4, 4, 3), i * 60, np.uint8)))
                files.append([str(path), rel])
            manifest = root / 'files.json'
            manifest.write_text(json.dumps(files), encoding='utf-8')
            out = root / 'out'
            extractor = mock.Mock()
            extractor.extract.side_effect = [np.array([0.0, 0.0]), None, np.array([1.0, 1.0])]
            messages = []
            argv = ['image_cluster.py', '--files', str(manifest), '--output', str(out),
                    '--algorithm', 'hdbscan', '--min-cluster-size', '5', '--device', 'cpu']
            with mock.patch.object(sys, 'argv', argv), \
                    mock.patch.object(image_cluster, 'bootstrap'), \
                    mock.patch.object(image_cluster, 'FeatureExtractor', return_value=extractor), \
                    mock.patch.object(image_cluster, 'generate_distribution_map'), \
                    mock.patch('purin_proto.emit', side_effect=messages.append):
                image_cluster.main()
            done = messages[-1]
            self.assertEqual(done['type'], 'done')
            self.assertEqual((done['clusters'], done['success_count'], done['fail_count'], done['total']),
                             (2, 2, 1, 3))
            copied = sorted(p.relative_to(out).as_posix() for p in out.rglob('*.png'))
            self.assertEqual(len(copied), 2)
            self.assertTrue(any(p.endswith('sub/2.png') for p in copied))
            self.assertEqual([p.name for p in out.rglob('*.tmp')], [])

    def test_fewer_than_two_images_fail_before_loading_the_model(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / 'files.json'
            manifest.write_text(json.dumps([['/data/a.png', '']]), encoding='utf-8')
            messages = []
            argv = ['image_cluster.py', '--files', str(manifest), '--output', directory, '--device', 'cpu']
            with mock.patch.object(sys, 'argv', argv), \
                    mock.patch.object(image_cluster, 'bootstrap'), \
                    mock.patch.object(image_cluster, 'FeatureExtractor') as extractor, \
                    mock.patch('purin_proto.emit', side_effect=messages.append), \
                    self.assertRaises(SystemExit):
                image_cluster.main()
            extractor.assert_not_called()
            self.assertEqual(messages[-1], {'type': 'error', 'message': '有效图片不足 2 张，无法聚类'})

    def test_distribution_map_is_written_atomically(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            features = np.random.default_rng(0).normal(size=(3, 4))
            paths = []
            for i in range(3):
                path = out / f'{i}.png'
                Image.new('RGB', (8, 8), (i * 80, 0, 0)).save(path)
                paths.append(str(path))
            with mock.patch.object(image_cluster, 'log'):
                image_cluster.generate_distribution_map(features, np.array([0, 0, 1]), paths, str(out))
            with Image.open(out / 'cluster_distribution.png') as im:
                self.assertEqual(im.format, 'PNG')
            self.assertEqual([p.name for p in out.iterdir() if p.suffix == '.tmp'], [])


if __name__ == '__main__':
    unittest.main()
