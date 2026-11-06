#!/usr/bin/env python3
"""Install KohakuGBC into PurinBox's separate env/gbc environment."""
import argparse
import subprocess
import sys
import venv
from pathlib import Path

REVISION = 'd90f360dbf561f63270522e44fa0e74ebed59c12'
ROOT = Path(__file__).resolve().parents[2]
ENV = ROOT / 'env' / 'gbc'


def main():
    if sys.version_info < (3, 11):
        raise SystemExit('KohakuGBC requires Python 3.11 or newer.')
    parser = argparse.ArgumentParser()
    parser.add_argument('--env', type=Path, default=ENV)
    target = parser.parse_args().env
    python = target / ('Scripts/python.exe' if sys.platform == 'win32' else 'bin/python')
    if not python.exists():
        venv.EnvBuilder(with_pip=True).create(target)
    subprocess.run([str(python), '-m', 'ensurepip'], check=True)
    archive = f'https://github.com/KohakuBlueleaf/KohakuGBC/archive/{REVISION}.zip'
    subprocess.run([str(python), '-m', 'pip', 'install',
                    f'kohakugbc-data @ {archive}#subdirectory=packages/kohakugbc-data',
                    f'kohakugbc[torch] @ {archive}'], check=True)
    subprocess.run([str(python), '-c', 'import kohakugbc, kohakugbc_data, torch; print("KohakuGBC environment ready")'], check=True)


if __name__ == '__main__':
    main()
