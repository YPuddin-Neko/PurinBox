#!/usr/bin/env python3
"""Prepare the managed GBC environment and local models for a tagging task."""
from collections import deque
import io
import time
import argparse
import json
import subprocess
import sys
import venv
from pathlib import Path

REVISION = 'd90f360dbf561f63270522e44fa0e74ebed59c12'
ROOT = Path(__file__).resolve().parents[2]
ENV = ROOT / 'env' / 'gbc'
PROBE = 'import kohakugbc, kohakugbc_data, torch, torchvision, transformers, sentence_transformers; from kohakugbc.machine.build import build_parts; from kohakugbc.engines.torch.entries import detector, embedder'
MODELS = json.loads(Path(__file__).with_name('gbc_models.json').read_text(encoding='utf-8'))


def emit(message):
    print(json.dumps({'type': 'log', 'message': message}, ensure_ascii=False), flush=True)


def progress(message, percent=0, filename='', speed_mbps=0):
    print(json.dumps({'type': 'download', 'status': 'downloading', 'message': message,
                      'percent': percent, 'filename': filename, 'speed_mbps': speed_mbps}, ensure_ascii=False), flush=True)


def run(command, *, forward_events=False):
    tail = deque(maxlen=6)
    child_error = None
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace') as process:
        for line in process.stdout:
            line = line.strip()
            if not line:
                continue
            tail.append(line)
            if not forward_events:
                continue
            try:
                payload = json.loads(line)
            except json.JSONDecodeError:
                continue
            if not isinstance(payload, dict):
                continue
            if payload.get('type') == 'error':
                child_error = payload.get('message')
            elif payload.get('type') in ('log', 'download'):
                print(json.dumps(payload, ensure_ascii=False), flush=True)
        if process.wait():
            raise RuntimeError(child_error or 'GBC 依赖安装失败：' + '\n'.join(tail)[-2000:])


def model_progress(name, completed, total, file_size):
    from tqdm.auto import tqdm

    class ModelProgress(tqdm):
        def __init__(self, *args, **kwargs):
            self.received = self.last_received = 0
            self.last_report = time.monotonic()
            kwargs.update(file=io.StringIO(), disable=False)
            super().__init__(*args, **kwargs)

        def display(self, *args, **kwargs):
            pass

        def report(self):
            now = time.monotonic()
            elapsed = now - self.last_report
            if elapsed < 0.25:
                return
            # Xet can receive bytes before writing them. Both counters describe the same file.
            file_done = min(file_size, max(self.n, self.initial + self.received))
            percent = min(99.9, (completed + file_done) / total * 100)
            speed = max(0, self.received - self.last_received) / elapsed / 1_000_000
            self.last_report, self.last_received = now, self.received
            progress(f'@pythonEnv.downloading|{name}', percent, name, speed)

        def update(self, n=1):
            super().update(n)
            self.report()

        def update_transfer(self, n):
            self.received += n
            self.report()

        def set_transfer_postfix_str(self, *args, **kwargs):
            pass

    return ModelProgress


def environment_ready(python):
    return python.is_file() and subprocess.run([str(python), '-c', PROBE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def prepare(target, device):
    python = target / ('Scripts/python.exe' if sys.platform == 'win32' else 'bin/python')
    if not python.is_file():
        emit('正在创建 GBC 环境')
        venv.EnvBuilder(with_pip=True).create(target)
    if not environment_ready(python):
        emit('正在安装 GBC 依赖')
        total = 3 if device == 'cuda' else 2
        progress(f'@pythonEnv.installingDep|pip|1|{total}')
        run([str(python), '-m', 'ensurepip'])
        if device == 'cuda':
            progress('@pythonEnv.installingDep|torch / torchvision|2|3', 100 / 3)
            run([str(python), '-m', 'pip', 'install', 'torch>=2.4', 'torchvision>=0.19', '--index-url', 'https://download.pytorch.org/whl/cu128'])
        archive = f'https://github.com/KohakuBlueleaf/KohakuGBC/archive/{REVISION}.zip'
        progress(f'@pythonEnv.installingDep|KohakuGBC|{total}|{total}', (total - 1) / total * 100)
        run([str(python), '-m', 'pip', 'install', '--only-binary=kohakugbc-data,kohakuvault',
             'kohakugbc-data==0.2.0', f'kohakugbc[torch] @ {archive}'])
        if not environment_ready(python):
            raise RuntimeError('GBC 依赖检查失败')
    if device == 'cuda' and subprocess.run([str(python), '-c', 'import torch; raise SystemExit(0 if torch.version.cuda else 1)'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        emit('正在安装 GBC GPU 依赖')
        progress('@pythonEnv.installingDep|torch / torchvision|1|1')
        run([str(python), '-m', 'pip', 'install', '--upgrade', 'torch', 'torchvision', '--index-url', 'https://download.pytorch.org/whl/cu128'])
    return python


def download_models(root, detector=None, embedder=None):
    from huggingface_hub import constants, hf_hub_download
    from huggingface_hub.errors import GatedRepoError

    # The HTTP fallback delivers progress once per read (10 MiB by default).
    constants.DOWNLOAD_CHUNK_SIZE = min(constants.DOWNLOAD_CHUNK_SIZE, 64 * 1024)
    for role, selected in [('detector', detector), ('embedder', embedder)]:
        choices = MODELS[role]
        if selected is None:
            selected = root / choices[0]['repo'].split('/')[1]
        model = next((item for item in choices if root / item['repo'].split('/')[1] == selected), None)
        # Custom checkpoints are loaded by the worker only when a new graph is needed.
        if model is None:
            continue
        files = model['files']
        pending = [file for file, size in files.items()
                   if not (selected / file).is_file() or (selected / file).stat().st_size != size]
        if not pending:
            continue
        total = sum(files.values())
        completed = total - sum(files[file] for file in pending)
        name = selected.name
        message = f'@pythonEnv.downloading|{name}'
        emit(f'正在下载 GBC 模型：{name}')
        progress(message, min(99.9, completed / total * 100), name)
        for file in pending:
            path = selected / file
            size = files[file]
            try:
                hf_hub_download(repo_id=model['repo'], filename=file, revision=model['revision'],
                                local_dir=str(selected), force_download=path.is_file(),
                                tqdm_class=model_progress(name, completed, total, size))
            except GatedRepoError as error:
                raise RuntimeError(f"下载 {model['repo']} 需要访问权限，请在 Hugging Face 获取授权并在设置中填写令牌") from error
            if not path.is_file() or path.stat().st_size != size:
                raise RuntimeError(f'GBC 模型文件不完整：{name}/{file}')
            completed += size
            progress(message, min(99.9, completed / total * 100) if completed < total else 100, name)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--env', type=Path, default=ENV)
    parser.add_argument('--models', type=Path)
    parser.add_argument('--detector', type=Path)
    parser.add_argument('--embedder', type=Path)
    parser.add_argument('--device', choices=['cpu', 'cuda', 'mps'], default='cpu')
    parser.add_argument('--download-models', action='store_true')
    args = parser.parse_args()
    if sys.version_info < (3, 11):
        raise RuntimeError('GBC 需要 Python 3.11 或更新版本')
    if args.download_models:
        download_models(args.models, args.detector, args.embedder)
        return
    python = prepare(args.env, args.device)
    if args.models:
        command = [str(python), __file__, '--download-models', '--models', str(args.models)]
        for flag, path in [('--detector', args.detector), ('--embedder', args.embedder)]:
            if path is not None:
                command.extend([flag, str(path)])
        run(command, forward_events=True)
    emit('GBC 环境已就绪，继续处理')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'type': 'error', 'message': str(error)}, ensure_ascii=False), flush=True)
        raise SystemExit(1)
