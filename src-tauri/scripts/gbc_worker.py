"""KohakuGBC subprocess; stdout carries JSON protocol only."""
import contextlib
import io
import json
import os
import sys
import tempfile
from pathlib import Path

from gbc_export import graph_document, layout_document, layout_from_graph

WIRE = sys.stdout

def emit(**value):
    WIRE.write(json.dumps(value, ensure_ascii=False) + '\n')
    WIRE.flush()


class Capture:
    def __init__(self):
        self.record = None
    def write(self, record):
        self.record = record
    def close(self):
        pass


def make_config(options):
    endpoint = options['api_endpoint'].rstrip('/')
    for suffix in ('/chat/completions', '/responses'):
        endpoint = endpoint.removesuffix(suffix)
    endpoint_spec = {'base_url': endpoint, 'max_inflight': options['concurrency'],
                     'headers': {'Authorization': 'Bearer ' + options['api_key']}}
    if options['request_interval_ms'] > 0:
        endpoint_spec['rate'] = [1000 / options['request_interval_ms'], 1]
    sampling = {'temperature': options['temperature']}
    if options['max_tokens'] > 0:
        sampling['max_tokens'] = options['max_tokens']
    if options['top_p'] > 0:
        sampling['top_p'] = options['top_p']
    return {
        'engines': {
            'vlm': {'entry': 'api', 'protocol': 'openai', 'flavour': 'openai',
                    'structured': 'prompt', 'model': options['model_name'],
                    'sampling': sampling, 'endpoints': [endpoint_spec]},
            'detect': {'entry': 'torch', 'model': options['detector_path'],
                       'devices': ['cuda:0' if options['device'] == 'cuda' else options['device']], 'dtype': 'float32'},
            'embed': {'entry': 'torch', 'model': options['embedder_path'],
                      'impl': 'st', 'devices': ['cpu'], 'dtype': 'float32'},
        },
        'controls': {'min_nodes': 1, 'max_nodes': options['max_nodes'],
                     'max_depth': options['max_depth'], 'max_rounds': options['max_depth'] + 1,
                     'max_side': options['image_size'], 'relational': 'both'},
        'machine': {'max_jobs': 1, 'vlm_waiting': options['concurrency']},
    }


def clean_json(text):
    text = text.strip()
    if text.startswith('```'):
        text = text.split('\n', 1)[1].rsplit('```', 1)[0].strip()
    return json.loads(text)


def render(graph, image, options):
    import base64
    graph_document(graph)
    kind = options['content_type']
    if kind == 'gbc':
        return json.dumps(graph, ensure_ascii=False, indent=2)
    import httpx
    w, h = image.size
    if kind == 'moss_layout':
        draft = layout_from_graph(graph, w, h)
        width, height = draft['width'], draft['height']
        contract = 'Return only the supplied MOSS Layout JSON draft. Keep all boxes, item order, canvas dimensions and coordinate fields unchanged. Classify each item as object, character, or text using the image. For a text item, add a text field with the exact visible characters; never invent unreadable text. You may improve captions without repeating them. No extra fields. Draft: ' + json.dumps(draft, ensure_ascii=False)
    else:
        contract = 'Write a coherent natural-language caption. Start with the scene, organize details by object, then relationships. Remove repeated descriptions. Do not output tags or JSON. ' + options.get('caption_prompt', '')
    buffer = io.BytesIO()
    preview = image.copy()
    preview.thumbnail((options['image_size'], options['image_size']))
    preview.save(buffer, format='PNG')
    endpoint = make_config(options)['engines']['vlm']['endpoints'][0]['base_url']
    request = {'model': options['model_name'], 'temperature': options['temperature'],
               'messages': [{'role':'system', 'content':contract},
                            {'role':'user', 'content':[{'type':'text','text':json.dumps(graph, ensure_ascii=False)},
                             {'type':'image_url','image_url':{'url':'data:image/png;base64,' + base64.b64encode(buffer.getvalue()).decode()}}]}]}
    if options.get('image_detail'):
        request['messages'][1]['content'][1]['image_url']['detail'] = options['image_detail']
    if options['max_tokens'] > 0:
        request['max_tokens'] = options['max_tokens']
    if options['top_p'] > 0:
        request['top_p'] = options['top_p']
    import time
    if options['request_interval_ms'] > 0:
        time.sleep(options['request_interval_ms'] / 1000)
    response = httpx.post(endpoint + '/chat/completions', headers={'Authorization':'Bearer ' + options['api_key']}, json=request, timeout=600)
    response.raise_for_status()
    text = response.json()['choices'][0]['message']['content'].strip()
    if not text:
        raise ValueError('描述结果为空')
    if kind == 'moss_layout':
        value = layout_document(clean_json(text))
        if (value['width'], value['height']) != (width, height):
            raise ValueError('Layout 画布与图片比例不符')
        if [item['bbox'] for item in value['items']] != [item['bbox'] for item in draft['items']]:
            raise ValueError('Layout 区域框与检测结果不一致')
        return json.dumps(value, ensure_ascii=False, indent=2)
    return json.dumps({'nl':text}, ensure_ascii=False, indent=2) if options['output_format'] == 'json' else text


def parse_graph_result(record):
    if record is None:
        raise ValueError('GBC 未返回图分析结果')
    if not record.ok:
        raise ValueError(str(record.failure))
    raw = record.outputs.get('graph')
    if raw is None or (isinstance(raw, str) and not raw.strip()):
        raise ValueError('GBC 未生成有效的图分析结果，无法细化这张图片')
    return graph_document(json.loads(raw))


def valid_existing(path, options):
    try:
        text = path.read_text(encoding='utf-8')
        kind = options['content_type']
        if kind == 'gbc':
            graph_document(json.loads(text))
        elif kind == 'moss_layout':
            layout_document(json.loads(text))
        elif options['output_format'] == 'json':
            value = json.loads(text)
            if not isinstance(value, dict) or set(value) != {'nl'} or not isinstance(value['nl'], str) or not value['nl'].strip():
                return False
        elif not text.strip():
            return False
        return True
    except (OSError, ValueError, TypeError, KeyError):
        return False


def analysis_identity(path, options):
    stat = path.stat()
    return {'size': stat.st_size, 'mtime_ns': stat.st_mtime_ns,
            'revision': 'd90f360dbf561f63270522e44fa0e74ebed59c12',
            'settings': {key: options[key] for key in ('model_name', 'api_endpoint', 'temperature', 'top_p', 'max_tokens', 'detector_path', 'embedder_path', 'max_nodes', 'max_depth', 'image_size')}}


def main():
    options = json.loads(sys.stdin.readline())
    os.environ['HF_HUB_OFFLINE'] = '1'
    os.environ['TRANSFORMERS_OFFLINE'] = '1'
    from PIL import Image, ImageOps
    config = make_config(options)
    machine = None
    request_count = [0]
    with contextlib.ExitStack() as stack, tempfile.TemporaryDirectory(prefix='purinbox-gbc-') as scratch:
        emit(type='ready')
        for line in sys.stdin:
            request = json.loads(line)
            if request.get('cmd') == 'quit':
                break
            try:
                existing = request.get('existing_path')
                if existing and valid_existing(Path(existing), options):
                    emit(type='skip')
                    continue
                path = Path(request['path'])
                identity = analysis_identity(path, options)
                with Image.open(path) as source:
                    image = ImageOps.exif_transpose(source).convert('RGB')
                graph = None
                existing_graph = path.with_suffix('.gbc.json')
                if existing_graph.is_file():
                    graph = graph_document(json.loads(existing_graph.read_text(encoding='utf-8')))
                    if graph.get('img_size') and list(graph['img_size']) != list(image.size):
                        raise ValueError('已有 GBC 数据的图片尺寸不匹配')
                if graph is None:
                    for name in ('detector_path', 'embedder_path'):
                        if not Path(options[name]).is_dir():
                            raise ValueError('请选择已下载的检测模型和嵌入模型目录')
                    if machine is None:
                        emit(type='log', message='正在加载检测与嵌入模型')
                        from togeari import SourceItem
                        from kohakugbc.machine.build import build_parts
                        parts = build_parts(config)
                        adapter = parts.slots['vlm'].adapter
                        prepare = adapter.prepare
                        def bounded_prepare(inputs, protocol):
                            request_count[0] += 1
                            if request_count[0] > options['max_requests']:
                                raise ValueError('已达到单图请求上限')
                            prepared = prepare(inputs, protocol)
                            if options.get('image_detail'):
                                for message in prepared.json.get('messages', []):
                                    if isinstance(message.get('content'), list):
                                        for part in message['content']:
                                            if part.get('type') == 'image_url':
                                                part['image_url']['detail'] = options['image_detail']
                            return prepared
                        adapter.prepare = bounded_prepare
                        machine = stack.enter_context(parts.machine)
                    working = str(Path(scratch) / 'image.png')
                    image.save(working)
                    emit(type='log', message='正在分析全图、区域和对象关系')
                    sink = Capture()
                    machine.run([SourceItem(request['path'], {'image': {'path': working, 'key':request['path']}, 'controls': config['controls']})], sink)
                    record = sink.record
                    graph = parse_graph_result(record)
                    graph['img_size'] = list(image.size)
                    graph['img_path'] = str(path)
                    graph['_purinbox'] = identity
                    emit(type='graph', graph=graph)
                else:
                    emit(type='log', message='复用已有 GBC 原始图')
                if request_count[0] >= options['max_requests'] and options['content_type'] != 'gbc':
                    raise ValueError('已达到单图请求上限；原始图已保留')
                emit(type='log', message='正在整理描述与导出内容')
                content = render(graph, image, options)
                if analysis_identity(path, options) != identity:
                    raise ValueError('图片已发生变化，请重新运行')
                emit(type='result', graph=graph, content=content)
            except Exception as error:
                emit(type='error', message=str(error))


if __name__ == '__main__':
    with contextlib.redirect_stdout(sys.stderr):
        try:
            main()
        except Exception as error:
            emit(type='error', message=str(error))
            sys.exit(1)
