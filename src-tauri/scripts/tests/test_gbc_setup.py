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

    def test_complete_models_skip_download_and_partial_models_resume(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for _, _, name, files in setup.MODELS:
                for file in files:
                    path = root / name / file
                    path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(b'x')
            hub = Mock()
            with patch.dict(sys.modules, {'huggingface_hub': hub}):
                setup.download_models(root); hub.snapshot_download.assert_not_called()
                repo, revision, name, files = setup.MODELS[0]
                (root / name / files[0]).unlink()
                def restore(**kwargs): (root / name / files[0]).write_bytes(b'x')
                hub.snapshot_download.side_effect = restore
                setup.download_models(root)
                hub.snapshot_download.assert_called_once_with(repo_id=repo, revision=revision, local_dir=str(root / name), allow_patterns=files, tqdm_class=ANY)

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

    def test_model_progress_uses_download_protocol_without_terminal_bars(self):
        output, terminal = io.StringIO(), io.StringIO()
        with patch('sys.stdout', output), patch('sys.stderr', terminal):
            progress = setup.model_progress('model')
            with progress(total=100, unit='B') as bar:
                bar.update(50)
                bar.display()
            with progress(total=10, unit='it') as count:
                count.update(1)
        events = [json.loads(line) for line in output.getvalue().splitlines()]
        self.assertTrue(any(event['percent'] == 50 for event in events))
        self.assertTrue(all(event['type'] == 'download' for event in events))
        self.assertEqual(terminal.getvalue(), '')

if __name__ == '__main__': unittest.main()
