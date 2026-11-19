#!/usr/bin/env python3
"""Build the pinned CPU-only whisper.cpp runtime shipped with Next Alpha.

No model or captured audio is bundled. Upstream code is MIT; retain its notices.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

REVISION = '4979e04f5dcaccb36057e059bbaed8a2f5288315'  # whisper.cpp v1.8.2
SHA256 = 'ec8e33f4c838ed459ad0a23da255c0262f150a6f54839a17e504ddf1495305d5'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cache', type=Path, default=Path('.cache/whisper'))
    args = parser.parse_args()
    cache = args.cache.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / 'source.tar.gz'
    if not archive.exists():
        partial = archive.with_suffix('.partial')
        try:
            with urllib.request.urlopen(f'https://github.com/ggml-org/whisper.cpp/archive/{REVISION}.tar.gz', timeout=60) as response, partial.open('wb') as out:
                shutil.copyfileobj(response, out)
            partial.replace(archive)
        finally:
            partial.unlink(missing_ok=True)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != SHA256:
        raise RuntimeError('whisper.cpp source integrity check failed')
    source = cache / f'whisper.cpp-{REVISION}'
    if not source.exists():
        with tarfile.open(archive) as package:
            package.extractall(cache, filter='data')
    build = cache / 'build'
    subprocess.run([
        'cmake', '-S', str(source), '-B', str(build),
        '-DCMAKE_BUILD_TYPE=Release', '-DBUILD_SHARED_LIBS=OFF',
        '-DGGML_NATIVE=OFF', '-DGGML_SSE42=OFF', '-DGGML_AVX=OFF',
        '-DGGML_AVX2=OFF', '-DGGML_FMA=OFF', '-DGGML_F16C=OFF',
        '-DGGML_OPENMP=OFF', '-DGGML_METAL=OFF',
        '-DGGML_CUDA=OFF', '-DWHISPER_BUILD_TESTS=OFF', '-DWHISPER_BUILD_EXAMPLES=ON',
        '-DCMAKE_POLICY_DEFAULT_CMP0091=NEW', '-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded',
    ], check=True)
    subprocess.run(['cmake', '--build', str(build), '--config', 'Release', '--target', 'whisper-cli', '-j', '4'], check=True)
    executable = 'whisper-cli.exe' if os.name == 'nt' else 'whisper-cli'
    candidate = next(p for p in [build / 'bin' / 'Release' / executable, build / 'bin' / executable] if p.is_file())
    output = cache / 'runtime'
    output.mkdir(exist_ok=True)
    shutil.copy2(candidate, output / executable)
    shutil.copy2(source / 'LICENSE', output / 'whisper-LICENSE.txt')
    print(f'whisper-runtime={output}')


if __name__ == '__main__':
    main()
