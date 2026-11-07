import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gbc_worker import make_config


class DeviceConfigTests(unittest.TestCase):
    def test_gpu_selection_has_explicit_index_and_other_devices_are_preserved(self):
        options = dict(api_endpoint='http://localhost/v1', api_key='', concurrency=1,
            request_interval_ms=-1, temperature=.3, max_tokens=-1, top_p=0,
            model_name='test', detector_path='detector', embedder_path='embedder',
            max_nodes=20, max_depth=3, image_size=1024)
        for device, expected in [('cuda', 'cuda:0'), ('cpu', 'cpu'), ('mps', 'mps')]:
            with self.subTest(device=device):
                config = make_config({**options, 'device': device})
                self.assertEqual(config['engines']['detect']['devices'], [expected])
                self.assertEqual(config['engines']['embed']['devices'], ['cpu'])


if __name__ == '__main__':
    unittest.main()
