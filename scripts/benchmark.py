#!/usr/bin/env python3
"""Benchmark the release executable; no third-party Python packages required."""
import argparse
import json
import pathlib
import platform
import statistics
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--seconds', type=float, default=1)
parser.add_argument('--runs', type=int, default=3)
parser.add_argument('--mode', choices=['fast', 'exact'], default='fast')
parser.add_argument('--binary', help='Optional executable for before/after comparisons')
args = parser.parse_args()
if args.runs < 1:
    parser.error('--runs must be positive')
root = pathlib.Path(__file__).resolve().parent.parent
binary = pathlib.Path(args.binary).resolve() if args.binary else root / 'target/release/dispatch-sat'
inputs = ['demo', *sorted(str(p.relative_to(root)) for p in (root / 'dataset').glob('*Синтетические*.csv'))]
print(f'{platform.system()} {platform.machine()} | {args.runs} runs | {args.seconds}s budget | {args.mode} | release')
print('| Input | Jobs | Median wall ms | Median encoding ms | Unassigned | Engineers (baseline) | km (baseline) | Globally proven |')
print('|---|---:|---:|---:|---:|---:|---:|---|')
for path in inputs:
    elapsed, encoding = [], []
    for _ in range(args.runs):
        start = time.perf_counter()
        run = subprocess.run([str(binary), 'solve', path, str(args.seconds), args.mode], cwd=root, capture_output=True, text=True, check=True)
        elapsed.append((time.perf_counter() - start) * 1000)
        response = json.loads(run.stdout)
        encoding.append(response['stats']['encoding_ms'])
    p, b = response['plan']['metrics'], response['baseline']['metrics']
    print(f'| {pathlib.Path(path).name} | {len(response["scenario"]["jobs"])} | {statistics.median(elapsed):.0f} | '
          f'{statistics.median(encoding):.0f} | {p["unassigned"]} | {p["engineers"]} ({b["engineers"]}) | '
          f'{p["distance_m"]/1000:.2f} ({b["distance_m"]/1000:.2f}) | {response["stats"]["optimal"]} |')
