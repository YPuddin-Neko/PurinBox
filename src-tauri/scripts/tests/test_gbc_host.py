"""Opt-in smoke test: PURINBOX_GBC_HOST_SMOKE=1, using env/gbc and local models."""
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest


@unittest.skipUnless(os.environ.get('PURINBOX_GBC_HOST_SMOKE') == '1', 'requires local GBC models and dependencies')
class LocalModelSmokeTests(unittest.TestCase):
    def test_worker_thread_failure_reports_error_and_exits_immediately(self):
        code = """
import sys, threading, time
sys.path.insert(0, sys.argv[1])
from gbc_worker import worker_thread_error
threading.excepthook = worker_thread_error
def fail():
    raise ValueError('injected CUDA initialization failure')
threading.Thread(target=fail, name='torch-cuda:0-all').start()
time.sleep(60)
"""
        scripts = Path(__file__).resolve().parents[1]
        result = subprocess.run([sys.executable, '-c', code, str(scripts)], capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, 1)
        event = json.loads(result.stdout)
        self.assertEqual(event['type'], 'error')
        self.assertIn('injected CUDA initialization failure', event['message'])

    def test_real_models_warm_up_in_worker_without_host(self):
        root = Path(__file__).resolve().parents[2]
        code = '''
import sys
from pathlib import Path
from unittest.mock import patch
root = Path(sys.argv[1])
sys.path.insert(0, str(root / 'scripts'))
from gbc_worker import make_config, build_analysis_parts
models = root.parent / 'models/gbc'
options = dict(api_endpoint='http://127.0.0.1:9/v1', api_key='', concurrency=1,
    request_interval_ms=-1, temperature=.3, max_tokens=-1, top_p=0, model_name='smoke',
    detector_path=str(models / 'owlv2-base-patch16-ensemble'),
    embedder_path=str(models / 'all-MiniLM-L6-v2'), device=sys.argv[2],
    max_nodes=20, max_depth=3, image_size=1024)
with patch('kohakugbc.engines.spawn.launch_all', side_effect=AssertionError('unexpected engine host')):
    parts, _ = build_analysis_parts(make_config(options))
with parts.machine:
    expected = 'cuda:0' if sys.argv[2] == 'cuda' else sys.argv[2]
    for slot in ('detect', 'embed'):
        unit, = parts.slots[slot].units
        assert unit.spec()['device'] == expected, (slot, unit.spec())
        assert unit.engine.isolation == 'thread'
        assert unit.engine.schedule == ('co_resident' if expected == 'cpu' else 'priority_streams')
        assert unit.load().extra['footprint']['warmup_batch'] > 0
        assert next(unit.model.model.parameters()).device.type == expected.split(':')[0]
    print('MACHINE_STARTED', flush=True)
print('MACHINE_STOPPED', flush=True)
'''
        device = os.environ.get('PURINBOX_GBC_TEST_DEVICE', 'cpu')
        result = subprocess.run([sys.executable, '-c', code, str(root), device],
            env={**os.environ, 'HF_HUB_OFFLINE': '1', 'TRANSFORMERS_OFFLINE': '1'},
            capture_output=True, text=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('MACHINE_STARTED', result.stdout)
        self.assertIn('MACHINE_STOPPED', result.stdout)
        events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
        self.assertEqual(events[-1]['type'], 'models_ready')
        stages = [event['message'] for event in events if event.get('model_stage')]
        self.assertEqual(stages[0], '正在启动 GBC 分析引擎')
        self.assertEqual(len(stages), 3, stages)
        expected = 'cuda:0' if device == 'cuda' else device
        self.assertIn(f'owlv2-base-patch16-ensemble（{expected}）', stages[1])
        self.assertIn(f'all-MiniLM-L6-v2（{expected}）', stages[2])


if __name__ == '__main__':
    unittest.main()
