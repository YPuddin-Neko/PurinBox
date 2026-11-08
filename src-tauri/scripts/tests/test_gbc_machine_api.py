"""Local HTTP integration for the installed GBC engine, without model downloads."""
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


@unittest.skipUnless(os.environ.get('PURINBOX_TEST_GBC_MACHINE') == '1', 'requires the GBC environment')
class MachineApiTests(unittest.TestCase):
    def test_retry_and_http_failure_through_real_machine(self):
        from PIL import Image
        from togeari import SourceItem
        from gbc_worker import AnalysisReplies, analyze_graph, build_analysis_parts
        for status in (200, 400):
            with self.subTest(status=status):
                requests = []
                class Handler(BaseHTTPRequestHandler):
                    def log_message(self, *args):
                        pass
                    def do_POST(self):
                        requests.append(json.loads(self.rfile.read(int(self.headers['Content-Length']))))
                        text = 'not JSON' if len(requests) == 1 else json.dumps({
                            'detailed_caption':'A blue square.', 'concise_caption':'A square.', 'elements':[]})
                        data = json.dumps({'choices':[{'message':{'content':text}, 'finish_reason':'stop'}]}).encode()
                        self.send_response(status)
                        self.send_header('Content-Type', 'application/json')
                        self.send_header('Content-Length', str(len(data)))
                        self.end_headers()
                        self.wfile.write(data)
                server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
                thread = threading.Thread(target=server.serve_forever, daemon=True)
                thread.start()
                try:
                    config = {'engines':{'vlm':{'entry':'api', 'protocol':'openai', 'flavour':'openai',
                        'structured':'prompt', 'model':'test', 'endpoints':[{'base_url':f'http://127.0.0.1:{server.server_port}/v1'}]}},
                        'controls':{'min_nodes':1, 'max_nodes':1, 'max_depth':0, 'max_rounds':0},
                        'machine':{'max_jobs':1, 'vlm_waiting':1}}
                    parts, failures = build_analysis_parts(config)
                    adapter = parts.slots['vlm'].adapter
                    guard = AnalysisReplies(adapter, {'max_tokens':-1, 'max_requests':20}, failures)
                    adapter.prepare, adapter.decode = guard.prepare, guard.decode
                    with tempfile.TemporaryDirectory() as folder, parts.machine as machine:
                        path = Path(folder) / 'image.png'
                        Image.new('RGB', (16,16), 'blue').save(path)
                        source = SourceItem(str(path), {'image':{'path':str(path), 'key':str(path)}, 'controls':config['controls']})
                        if status == 200:
                            self.assertTrue(analyze_graph(machine, source, guard, path.name)['vertices'])
                            self.assertEqual(len(requests), 2)
                            self.assertTrue(all('max_tokens' not in req for req in requests))
                            self.assertTrue(analyze_graph(machine, source, guard, path.name)['vertices'])
                            self.assertEqual(guard.count, 1)
                        else:
                            with self.assertRaisesRegex(ValueError, 'GBC 模型请求失败.*400'):
                                analyze_graph(machine, source, guard, path.name)
                finally:
                    server.shutdown()
                    server.server_close()
                    thread.join()


if __name__ == '__main__':
    unittest.main()
