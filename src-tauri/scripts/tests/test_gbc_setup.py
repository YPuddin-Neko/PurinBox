import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch, Mock, ANY
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gbc_setup as setup

class SetupTests(unittest.TestCase):
    def test_ready_environment_does_not_install(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            python = root / ('Scripts/python.exe' if sys.platform == 'win32' else 'bin/python')
            python.parent.mkdir(); python.touch()
            with patch.object(setup, 'environment_ready', return_value=True), patch.object(setup, 'run') as run:
                self.assertEqual(setup.prepare(root, 'cpu'), python)
                run.assert_not_called()

    def test_missing_dependencies_use_wheels_and_failed_install_can_retry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            python = root / ('Scripts/python.exe' if sys.platform == 'win32' else 'bin/python')
            python.parent.mkdir(); python.touch()
            with patch.object(setup, 'environment_ready', return_value=False), patch.object(setup, 'run', side_effect=RuntimeError('failed')):
                with self.assertRaises(RuntimeError): setup.prepare(root, 'cpu')
            with patch.object(setup, 'environment_ready', side_effect=[False, True]), patch.object(setup, 'run') as run:
                setup.prepare(root, 'cpu')
                self.assertIn('--only-binary=kohakugbc-data,kohakuvault', run.call_args.args[0])

    def tiny_catalog(self):
        return {role: [{**m, 'files': dict.fromkeys(m['files'], 1)} for m in models]
                for role, models in setup.MODELS.items()}

    def test_complete_models_skip_download_and_partial_models_resume(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(setup, 'MODELS', self.tiny_catalog()):
            root = Path(directory)
            defaults = [setup.MODELS[role][0] for role in ('detector', 'embedder')]
            for model in defaults:
                for file in model['files']:
                    path = root / model['repo'].split('/')[1] / file
                    path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'x')
            with patch('huggingface_hub.hf_hub_download') as download:
                setup.download_models(root)
                download.assert_not_called()
                model = defaults[0]
                folder = root / model['repo'].split('/')[1]
                missing = folder / 'config.json'
                missing.unlink()
                download.side_effect = lambda **kwargs: missing.write_bytes(b'x')
                setup.download_models(root)
                download.assert_called_once_with(repo_id=model['repo'], revision=model['revision'],
                    filename=missing.name, local_dir=str(folder), force_download=False, tqdm_class=ANY)
                missing.write_bytes(b'bad')
                setup.download_models(root)
                self.assertTrue(download.call_args.kwargs['force_download'])

    def test_only_selected_models_download_and_complete_pairs_are_reused(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(setup, 'MODELS', self.tiny_catalog()):
            root = Path(directory)
            def write_file(**args):
                file = Path(args['local_dir']) / args['filename']
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(b'x')
            for detector in setup.MODELS['detector']:
                for embedder in setup.MODELS['embedder']:
                    with self.subTest(detector=detector['label'], embedder=embedder['label']):
                        folder = Path(tempfile.mkdtemp(dir=root))
                        paths = [folder / model['repo'].split('/')[1] for model in (detector, embedder)]
                        expected = [(m['repo'], f) for m in (detector, embedder) for f in m['files']]
                        with patch('huggingface_hub.hf_hub_download', side_effect=write_file) as download, patch('sys.stdout', io.StringIO()):
                            setup.download_models(folder, *paths)
                            setup.download_models(folder, *paths)
                            self.assertEqual([(c.kwargs['repo_id'], c.kwargs['filename'])
                                              for c in download.call_args_list], expected)

    def test_child_json_logs_are_not_double_encoded(self):
        process = Mock()
        process.stdout = ['{"type":"log","message":"下载模型"}\n', 'pip message\n']
        process.wait.return_value = 0
        context = Mock(); context.__enter__ = Mock(return_value=process); context.__exit__ = Mock(return_value=False)
        output = io.StringIO()
        with patch.object(setup.subprocess, 'Popen', return_value=context), patch('sys.stdout', output): setup.run(['python'], forward_events=True)
        self.assertEqual([json.loads(line)['message'] for line in output.getvalue().splitlines()], ['下载模型'])

    def test_successful_pip_output_is_not_written_to_task_logs(self):
        output = io.StringIO()
        command = [sys.executable, '-c', 'print("Collecting torch"); print("Requirement already satisfied: pip"); print("Looking in indexes: https://example.test")']
        with patch('sys.stdout', output): setup.run(command)
        self.assertEqual(output.getvalue(), '')

    def test_pip_failure_preserves_reason_without_streaming_output(self):
        output = io.StringIO()
        command = [sys.executable, '-c', 'print("Collecting torch"); print("ERROR: No matching distribution found for torch"); raise SystemExit(1)']
        with patch('sys.stdout', output), self.assertRaisesRegex(RuntimeError, 'No matching distribution found for torch'):
            setup.run(command)
        self.assertEqual(output.getvalue(), '')

    def test_child_download_events_keep_progress_and_error_is_reported_once(self):
        output = io.StringIO()
        event = dict(type='download', status='downloading', message='model', percent=42, speed_mbps=3.5)
        error = dict(type='error', message='GBC 模型文件不完整')
        code = f'import json; print(json.dumps({event!r})); print(json.dumps({error!r})); raise SystemExit(1)'
        with patch('sys.stdout', output), self.assertRaisesRegex(RuntimeError, 'GBC 模型文件不完整'):
            setup.run([sys.executable, '-c', code], forward_events=True)
        self.assertEqual([json.loads(line) for line in output.getvalue().splitlines()], [event])

    def test_xet_network_progress_precedes_disk_writes_and_keeps_one_model_total(self):
        from types import SimpleNamespace
        from huggingface_hub.utils._xet_progress_reporting import XetDownloadProgressReporter
        output, terminal = io.StringIO(), io.StringIO()
        def report(written, received):
            return SimpleNamespace(total_bytes_completed=written, total_transfer_bytes_completed=received,
                total_transfer_bytes_completion_rate=1, total_bytes_completion_rate=1, total_bytes=800)
        with patch('sys.stdout', output), patch('sys.stderr', terminal), patch.object(setup.time, 'monotonic', return_value=0) as clock:
            with XetDownloadProgressReporter(reconstruction_desc='weight: reconstructing file',
                    transfer_desc='weight: downloading bytes', total=800, log_level=20, name='test',
                    tqdm_class=setup.model_progress('test', 200, 1000, 800)) as reporter:
                clock.return_value = 1
                reporter.update_progress(report(0, 400))
                clock.return_value = 2
                reporter.update_progress(report(400, 400))
                clock.return_value = 3
                reporter.update_progress(report(800, 800))
        events = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertEqual([e['percent'] for e in events], [60, 60, 99.9])
        self.assertGreater(events[0]['speed_mbps'], 0)
        self.assertEqual(terminal.getvalue(), '')

if __name__ == '__main__': unittest.main()
