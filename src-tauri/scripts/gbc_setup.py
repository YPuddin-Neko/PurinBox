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
PROBE = 'import kohakugbc, kohakugbc_data, torch, torchvision, transformers, sentence_transformers; from kohakugbc.machine.build import build_parts'
MODELS = [
    ('google/owlv2-base-patch16-ensemble', 'cfd3195ba4ea9592eec887ded089f4c08eff231d',
     'owlv2-base-patch16-ensemble', ['config.json', 'preprocessor_config.json', 'tokenizer_config.json', 'special_tokens_map.json', 'vocab.json', 'merges.txt', 'added_tokens.json', 'model.safetensors']),
    ('sentence-transformers/all-MiniLM-L6-v2', '1110a243fdf4706b3f48f1d95db1a4f5529b4d41',
     'all-MiniLM-L6-v2', ['config.json', 'config_sentence_transformers.json', 'sentence_bert_config.json', 'modules.json', '1_Pooling/config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'vocab.txt', 'model.safetensors']),
]


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


def model_progress(name):
    from tqdm.auto import tqdm

    class ModelProgress(tqdm):
        def __init__(self, *args, **kwargs):
            kwargs.update(file=io.StringIO(), disable=False, mininterval=0.25)
            super().__init__(*args, **kwargs)

        def display(self, *args, **kwargs):
            if self.unit != 'B' or self.desc == 'Downloading bytes':
                return
            elapsed = time.time() - getattr(self, 'start_t', time.time())
            speed = max(0, self.n - self.initial) / elapsed / 1_000_000 if elapsed > 0 else 0
            percent = min(100, max(0, self.n / self.total * 100)) if self.total else 0
            progress(f'@pythonEnv.downloading|{name}', percent, name, speed)

    return ModelProgress


def environment_ready(python):
    return python.is_file() and subprocess.run([str(python), '-c', PROBE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0


def model_ready(folder, files):
    return all((folder / name).is_file() and (folder / name).stat().st_size > 0 for name in files)


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


def download_models(root):
    from huggingface_hub import snapshot_download
    for repo, revision, name, files in MODELS:
        folder = root / name
        if model_ready(folder, files):
            continue
        emit(f'正在下载 GBC 模型：{name}')
        progress(f'@pythonEnv.downloading|{name}', filename=name)
        snapshot_download(repo_id=repo, revision=revision, local_dir=str(folder), allow_patterns=files, tqdm_class=model_progress(name))
        if not model_ready(folder, files):
            raise RuntimeError(f'GBC 模型文件不完整：{name}')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--env', type=Path, default=ENV)
    parser.add_argument('--models', type=Path)
    parser.add_argument('--device', choices=['cpu', 'cuda', 'mps'], default='cpu')
    parser.add_argument('--download-models', action='store_true')
    args = parser.parse_args()
    if sys.version_info < (3, 11):
        raise RuntimeError('GBC 需要 Python 3.11 或更新版本')
    if args.download_models:
        download_models(args.models)
        return
    python = prepare(args.env, args.device)
    if args.models:
        run([str(python), __file__, '--download-models', '--models', str(args.models)], forward_events=True)
    emit('GBC 环境已就绪，继续处理')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'type': 'error', 'message': str(error)}, ensure_ascii=False), flush=True)
        raise SystemExit(1)
