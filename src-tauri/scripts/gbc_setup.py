#!/usr/bin/env python3
"""Prepare the managed GBC environment and local models for a tagging task."""
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


def run(command):
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding='utf-8', errors='replace') as process:
        for line in process.stdout:
            if line.strip():
                try:
                    payload = json.loads(line)
                except json.JSONDecodeError:
                    payload = None
                emit(payload.get('message', line.strip()) if isinstance(payload, dict) else line.strip())
        if process.wait():
            raise RuntimeError('GBC 依赖安装失败')


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
        run([str(python), '-m', 'ensurepip'])
        if device == 'cuda':
            run([str(python), '-m', 'pip', 'install', 'torch>=2.4', 'torchvision>=0.19', '--index-url', 'https://download.pytorch.org/whl/cu128'])
        archive = f'https://github.com/KohakuBlueleaf/KohakuGBC/archive/{REVISION}.zip'
        run([str(python), '-m', 'pip', 'install', '--only-binary=kohakugbc-data,kohakuvault',
             'kohakugbc-data==0.2.0', f'kohakugbc[torch] @ {archive}'])
        if not environment_ready(python):
            raise RuntimeError('GBC 依赖检查失败')
    if device == 'cuda' and subprocess.run([str(python), '-c', 'import torch; raise SystemExit(0 if torch.version.cuda else 1)'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        emit('正在安装 GBC GPU 依赖')
        run([str(python), '-m', 'pip', 'install', '--upgrade', 'torch', 'torchvision', '--index-url', 'https://download.pytorch.org/whl/cu128'])
    return python


def download_models(root):
    from huggingface_hub import snapshot_download
    for repo, revision, name, files in MODELS:
        folder = root / name
        if model_ready(folder, files):
            continue
        emit(f'正在下载 GBC 模型：{name}')
        snapshot_download(repo_id=repo, revision=revision, local_dir=str(folder), allow_patterns=files)
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
        run([str(python), __file__, '--download-models', '--models', str(args.models)])
    emit('GBC 环境已就绪，继续处理')


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        print(json.dumps({'type': 'error', 'message': str(error)}, ensure_ascii=False), flush=True)
        raise SystemExit(1)
