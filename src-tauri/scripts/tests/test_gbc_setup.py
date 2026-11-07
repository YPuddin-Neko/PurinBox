import io
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch, Mock
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
                hub.snapshot_download.assert_called_once_with(repo_id=repo, revision=revision, local_dir=str(root / name), allow_patterns=files)

    def test_child_json_logs_are_not_double_encoded(self):
        process = Mock()
        process.stdout = ['{"type":"log","message":"下载模型"}\n', 'pip message\n']
        process.wait.return_value = 0
        context = Mock(); context.__enter__ = Mock(return_value=process); context.__exit__ = Mock(return_value=False)
        output = io.StringIO()
        with patch.object(setup.subprocess, 'Popen', return_value=context), patch('sys.stdout', output): setup.run(['python'])
        self.assertEqual([json.loads(line)['message'] for line in output.getvalue().splitlines()], ['下载模型', 'pip message'])

if __name__ == '__main__': unittest.main()
