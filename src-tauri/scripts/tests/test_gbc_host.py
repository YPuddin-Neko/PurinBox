"""Opt-in smoke test: PURINBOX_GBC_HOST_SMOKE=1, using env/gbc and local models."""
import os
from pathlib import Path
import subprocess
import sys
import unittest


@unittest.skipUnless(os.environ.get('PURINBOX_GBC_HOST_SMOKE') == '1', 'requires local GBC models and dependencies')
class HostSmokeTests(unittest.TestCase):
    def test_host_thread_failure_reports_error_and_exits_immediately(self):
        code = """
import threading, time
from kohakugbc.cli.main import _purinbox_host_thread_error
threading.excepthook = _purinbox_host_thread_error
def fail():
    raise ValueError('injected CUDA initialization failure')
threading.Thread(target=fail, name='torch-cuda:0-all').start()
time.sleep(60)
"""
        result = subprocess.run([sys.executable, '-c', code], capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 1)
        import json
        event = json.loads(result.stdout)
        self.assertEqual(event['type'], 'error')
        self.assertIn('injected CUDA initialization failure', event['message'])

    def test_real_detector_and_embedder_host_starts_and_stops(self):
        root = Path(__file__).resolve().parents[2]
        code = '''
import sys
from pathlib import Path
root = Path(sys.argv[1])
sys.path.insert(0, str(root / 'scripts'))
from gbc_worker import make_config
from kohakugbc.machine.build import build_parts
models = root.parent / 'models/gbc'
options = dict(api_endpoint='http://127.0.0.1:9/v1', api_key='', concurrency=1,
    request_interval_ms=-1, temperature=.3, max_tokens=-1, top_p=0, model_name='smoke',
    detector_path=str(models / 'owlv2-base-patch16-ensemble'),
    embedder_path=str(models / 'all-MiniLM-L6-v2'), device='cpu',
    max_nodes=20, max_depth=3, image_size=1024)
parts = build_parts(make_config(options))
with parts.machine:
    print('MACHINE_STARTED', flush=True)
print('MACHINE_STOPPED', flush=True)
'''
        result = subprocess.run([sys.executable, '-c', code, str(root)],
            env={**os.environ, 'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'},
            capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('MACHINE_STARTED', result.stdout)
        self.assertIn('MACHINE_STOPPED', result.stdout)


if __name__ == '__main__':
    unittest.main()
