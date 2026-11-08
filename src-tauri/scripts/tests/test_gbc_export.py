import copy
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gbc_export import graph_document, layout_document, layout_from_graph


def graph():
    edge = {'source': '', 'target': 'cup', 'text': 'cup'}
    return {'vertices': [
        {'vertex_id': '', 'label': 'image', 'bbox': dict(left=0, top=0, right=1, bottom=1),
         'descs': [{'label': 'detail', 'text': 'A cup on a desk.'}], 'out_edges': [edge], 'in_edges': []},
        {'vertex_id': 'cup', 'label': 'entity', 'bbox': dict(left=.1, top=.2, right=.5, bottom=.8),
         'descs': [{'label': 'detail', 'text': 'A blue cup.'}], 'out_edges': [], 'in_edges': [edge]},
    ]}


class ExportContractTests(unittest.TestCase):
    def test_empty_graph_result_is_reported_before_json_decode(self):
        import json
        from types import SimpleNamespace
        from gbc_worker import parse_graph_result
        for raw in (None, '', '  '):
            with self.subTest(raw=raw), self.assertRaisesRegex(ValueError, '未生成有效的图分析结果'):
                parse_graph_result(SimpleNamespace(ok=True, outputs={'graph': raw}))
        self.assertEqual(parse_graph_result(SimpleNamespace(ok=True, outputs={'graph': json.dumps(graph())})), graph())
        with self.assertRaisesRegex(ValueError, 'upstream failure'):
            parse_graph_result(SimpleNamespace(ok=False, failure='upstream failure'))

    def test_graph_preserves_metadata(self):
        value = graph()
        value['source_metadata'] = {'seed': 18446744073709551615}
        original = copy.deepcopy(value)
        self.assertEqual(graph_document({'graph': value}), original)

    def test_invalid_graphs(self):
        cases = []
        value = graph(); value['vertices'][1]['vertex_id'] = ''; cases.append(value)
        value = graph(); value['vertices'][1]['bbox']['right'] = 2; cases.append(value)
        value = graph(); value['vertices'][1]['in_edges'] = []; cases.append(value)
        value = graph(); value['vertices'][0]['out_edges'] = []; value['vertices'][1]['in_edges'] = []; cases.append(value)
        value = graph(); value['vertices'][0]['out_edges'][0]['target'] = 'missing'; cases.append(value)
        value = graph(); edge = {'source':'cup', 'target':'', 'text':'cycle'}; value['vertices'][1]['out_edges']=[edge];value['vertices'][0]['in_edges']=[edge]; cases.append(value)
        for value in cases:
            with self.subTest(value=value), self.assertRaises(ValueError):
                graph_document(value)

    def test_layout_contract(self):
        value = dict(width=1024, height=1024, bbox_coord_space='abs', bbox_format='xyxy',
                     summary={'canvas':'single', 'caption':'A sign.'},
                     items=[{'label':'text', 'caption':'A blue sign.', 'text':'Hello\n世界', 'bbox':[10,20,200,300]}])
        self.assertEqual(layout_document(value), value)
        for change in ({'width':1023}, {'extra':True}, {'items':[]}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                layout_document({**value, **change})
        invalid = copy.deepcopy(value); del invalid['items'][0]['text']
        with self.assertRaises(ValueError):
            layout_document(invalid)

    def test_layout_keeps_image_ratio_and_detected_boxes(self):
        layout = layout_from_graph(graph(), 101, 57)
        self.assertEqual(layout['width'] / layout['height'], 101 / 57)
        self.assertEqual(layout['width'] % 16, 0)
        self.assertEqual(layout['height'] % 16, 0)
        self.assertEqual(layout['items'][0]['bbox'], [layout['width']*.1, layout['height']*.2, layout['width']*.5, layout['height']*.8])

    def test_existing_result_must_match_export_kind(self):
        import tempfile
        from gbc_worker import valid_existing
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'a.json'
            path.write_text('{"tags":"blue"}')
            self.assertFalse(valid_existing(path, {'content_type':'gbc'}))
            self.assertFalse(valid_existing(path, {'content_type':'caption','output_format':'json'}))
            path.write_text('{"nl":"A blue cup."}')
            self.assertTrue(valid_existing(path, {'content_type':'caption','output_format':'json'}))
            self.assertFalse(valid_existing(path, {'content_type':'moss_layout'}))

    def test_render_caption_and_layout_response_contract(self):
        import json
        from types import SimpleNamespace
        from unittest.mock import Mock, patch
        from PIL import Image
        from gbc_worker import render
        options = dict(api_endpoint='http://127.0.0.1:1/v1', api_key='fixture', concurrency=1,
                       request_interval_ms=-1, temperature=.2, max_tokens=512, top_p=0,
                       model_name='fixture', detector_path='missing', embedder_path='missing',
                       device='cpu', max_nodes=20, max_depth=3, image_size=1024,
                       content_type='caption', output_format='json')
        image = Image.new('RGB', (32, 16))
        response = Mock()
        post = Mock(return_value=response)
        with patch.dict(sys.modules, {'httpx': SimpleNamespace(post=post)}):
            response.json.return_value = {'choices':[{'message':{'content':'A blue cup on a desk.'}}]}
            self.assertEqual(json.loads(render(graph(), image, options)), {'nl':'A blue cup on a desk.'})
            options['content_type'] = 'moss_layout'
            draft = layout_from_graph(graph(), *image.size)
            response.json.return_value = {'choices':[{'message':{'content':json.dumps(draft)}}]}
            self.assertEqual(json.loads(render(graph(), image, options)), draft)
            draft['items'][0]['bbox'][0] += 1
            response.json.return_value = {'choices':[{'message':{'content':json.dumps(draft)}}]}
            with self.assertRaisesRegex(ValueError, '区域框'):
                render(graph(), image, options)

    def test_worker_reuses_raw_graph_without_loading_models(self):
        import json
        import subprocess
        import tempfile
        from PIL import Image
        from gbc_worker import analysis_identity
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'a.png'
            Image.new('RGB', (32, 16)).save(path)
            options = dict(api_endpoint='http://127.0.0.1:1/v1', api_key='fixture', concurrency=1,
                           request_interval_ms=-1, temperature=.2, max_tokens=512, top_p=0,
                           model_name='fixture', detector_path='missing', embedder_path='missing',
                           device='cpu', max_nodes=20, max_depth=3, max_requests=64, image_size=1024,
                           content_type='gbc', output_format='json')
            value = graph(); value['_purinbox'] = analysis_identity(path, options)
            path.with_suffix('.gbc.json').write_text(json.dumps(value))
            commands = [options, {'cmd':'process','path':str(path)}, {'cmd':'quit'}]
            result = subprocess.run([sys.executable, str(Path(__file__).resolve().parents[1]/'gbc_worker.py')],
                                    input=''.join(json.dumps(c)+'\n' for c in commands), capture_output=True, text=True, timeout=15)
            self.assertEqual(result.returncode, 0, result.stderr)
            events = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(events[0]['type'], 'ready')
            output = next(e for e in events if e['type'] == 'result')
            self.assertEqual(json.loads(output['content']), value)


if __name__ == '__main__':
    unittest.main()
