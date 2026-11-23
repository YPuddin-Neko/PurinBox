"""Exercise the real GBC scheduler and subprocess protocol against a local API."""
import base64
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


@unittest.skipUnless(os.environ.get('PURINBOX_TEST_GBC_MACHINE') == '1', 'requires the GBC environment')
class ConcurrentWorkerTests(unittest.TestCase):
    def test_parallel_images_retry_isolation_and_restart(self):
        from PIL import Image
        scripts = Path(__file__).resolve().parents[1]
        lock = threading.Lock()
        state = {'active':0, 'peak':0, 'red_calls':0}
        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                url = next(part['image_url']['url'] for msg in body['messages'] if isinstance(msg['content'], list)
                           for part in msg['content'] if part['type'] == 'image_url')
                with Image.open(io.BytesIO(base64.b64decode(url.split(',',1)[1]))) as image:
                    rgb = image.convert('RGB').getpixel((0,0))
                colour = ('red','green','blue')[rgb.index(max(rgb))]
                with lock:
                    state['active'] += 1
                    state['peak'] = max(state['peak'], state['active'])
                    if colour == 'red': state['red_calls'] += 1
                    attempt = state['red_calls']
                try:
                    time.sleep(.3 if colour == 'red' else .15)
                    text = 'malformed' if colour == 'red' and attempt == 1 else json.dumps({
                        'detailed_caption':f'A {colour} square.', 'concise_caption':f'{colour} square.', 'elements':[]})
                    data = json.dumps({'choices':[{'message':{'content':text}, 'finish_reason':'stop'}]}).encode()
                    self.send_response(400 if colour == 'green' else 200)
                    self.send_header('Content-Type','application/json')
                    self.send_header('Content-Length',str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                finally:
                    with lock: state['active'] -= 1
        server = ThreadingHTTPServer(('127.0.0.1',0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                for colour in ('red','blue','green'):
                    Image.new('RGB',(16,16),colour).save(root / f'{colour}.png')
                harness = root / 'worker.py'
                harness.write_text('''import sys
sys.path.insert(0, sys.argv[1])
import gbc_worker as worker
protocol_input = sys.stdin
models_ready = False
worker_ready = False
original_emit = worker.emit
def emit(**event):
    global models_ready, worker_ready
    if event['type'] == 'models_ready':
        models_ready = True
    if event['type'] == 'ready':
        assert models_ready and event['mode'] == 'analysis'
        worker_ready = True
    original_emit(**event)
worker.emit = emit

class CommandPipe:
    readline = protocol_input.readline
    def __iter__(self):
        assert worker_ready, 'read first image command before engine ready'
        yield from protocol_input
sys.stdin = CommandPipe()

original = worker.make_config
def config(options):
    value = original(options)
    del value['engines']['detect']
    del value['engines']['embed']
    value['controls'].update(min_nodes=1, max_nodes=1, max_depth=0, max_rounds=0)
    return value
worker.make_config = config
worker.main()
''')
                options = dict(parallel_analysis=True, concurrency=2, max_requests=3, max_tokens=-1,
                    api_endpoint=f'http://127.0.0.1:{server.server_port}/v1', api_key='fixture', model_name='fixture',
                    temperature=.1, top_p=0, request_interval_ms=0, detector_path='', embedder_path='',
                    device='cpu', image_size=64, max_nodes=1, max_depth=0, content_type='gbc', output_format='json')
                for concurrency in (1, 2, 4):
                    state.update(active=0, peak=0, red_calls=0)
                    options['concurrency'] = concurrency
                    process = subprocess.Popen([sys.executable,'-u',str(harness),str(scripts)], stdin=subprocess.PIPE,
                                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
                    messages = queue.Queue()
                    logs = []
                    reader = threading.Thread(target=lambda: [messages.put(json.loads(line)) for line in process.stdout], daemon=True)
                    reader.start()
                    def send(value):
                        process.stdin.write(json.dumps(value)+'\n');process.stdin.flush()
                    def receive_terminal(count):
                        results = {}
                        deadline = time.monotonic()+40
                        while len(results)<count:
                            msg = messages.get(timeout=max(.1, deadline-time.monotonic()))
                            if msg['type'] == 'log': logs.append(msg['message'])
                            if msg['type'] in ('result','error'):
                                self.assertIn('id',msg,msg)
                                self.assertNotIn(msg['id'],results)
                                results[msg['id']] = msg
                        return results
                    try:
                        send(options)
                        startup = []
                        while True:
                            msg = messages.get(timeout=15)
                            startup.append(msg['type'])
                            if msg['type'] == 'log': logs.append(msg['message'])
                            if msg['type'] == 'ready': break
                            self.assertIn(msg['type'], ('log', 'models_ready'), msg)
                        self.assertEqual(msg['mode'], 'analysis')
                        self.assertIn('models_ready', startup)
                        self.assertEqual(state['peak'], 0)
                        send({'cmd':'process','id':'1','path':str(root/'red.png')})
                        send({'cmd':'process','id':'2','path':str(root/'blue.png')})
                        for index in range(3, concurrency + 1):
                            send({'cmd':'process','id':str(index),'path':str(root/'blue.png')})
                        results = receive_terminal(max(2, concurrency))
                        self.assertEqual(results['1']['type'],'result',results)
                        self.assertIn('red square',results['1']['content'])
                        self.assertIn('blue square',results['2']['content'])
                        self.assertEqual(state['peak'],concurrency)
                        self.assertEqual(state['red_calls'],2)
                        send({'cmd':'process','id':'11','path':str(root/'green.png')})
                        send({'cmd':'process','id':'12','path':str(root/'blue.png')})
                        results = receive_terminal(2)
                        self.assertEqual(results['11']['type'],'error')
                        self.assertIn('400',results['11']['message'])
                        self.assertEqual(results['12']['type'],'result')
                        self.assertEqual(logs.count('正在分析全图、区域和对象关系'),1)
                        self.assertEqual(logs.count('正在启动 GBC 分析引擎'),1)
                        # Cancellation can kill the one worker while several jobs are active.
                        send({'cmd':'process','id':'5','path':str(root/'red.png')})
                        send({'cmd':'process','id':'6','path':str(root/'blue.png')})
                    finally:
                        process.terminate()
                        process.wait(timeout=10)
                        for pipe in (process.stdin,process.stdout,process.stderr): pipe.close()
                        reader.join(timeout=2)
        finally:
            server.shutdown();server.server_close();thread.join()

if __name__ == '__main__':
    unittest.main()
