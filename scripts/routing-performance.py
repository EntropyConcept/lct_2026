#!/usr/bin/env python3
"""Measure cold/warm road preparation against a local MOTIS and compare snapshots.

Example: python3 scripts/routing-performance.py --output /tmp/road-after \
    --reference /tmp/road-before/report.json
Each invocation uses an empty temporary cache. Public engineers are excluded;
this measures roads, not timetable preparation or dispatch optimization.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/dispatch-sat')
    parser.add_argument('--scenario', type=Path, default=ROOT / 'demo.json')
    parser.add_argument('--router', default='http://127.0.0.1:8081')
    parser.add_argument('--workers', type=int, choices=range(1, 17), default=4)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--reference', type=Path, help='report.json from an earlier run')
    args = parser.parse_args()
    scenario = json.loads(args.scenario.read_text())
    excluded = [e['id'] for e in scenario['engineers'] if e['transport'] == 'public']
    scenario['engineers'] = [e for e in scenario['engineers'] if e['transport'] != 'public']
    if not scenario['engineers']:
        parser.error('The scenario must contain at least one road engineer')
    scenario['routing'] = None
    # A fresh directory prevents accidentally mixing reports from different runs.
    args.output.mkdir(parents=True, exist_ok=False)
    source = args.output.resolve() / 'input.json'
    source.write_text(json.dumps(scenario, ensure_ascii=False))
    elapsed = {}
    with tempfile.TemporaryDirectory(prefix='dispatch-road-perf-') as cache:
        env = dict(os.environ, DISPATCH_ROAD_BACKEND='motis',
                   DISPATCH_MOTIS_URL=args.router, DISPATCH_ROUTING_CACHE=cache,
                   DISPATCH_ROAD_WORKERS=str(args.workers))
        for stage in ('cold', 'warm'):
            started = time.perf_counter()
            result = subprocess.run([str(args.binary.resolve()), 'route', str(source),
                                     '--confirmed'], cwd=ROOT, env=env,
                                    capture_output=True, text=True, timeout=1250)
            elapsed[stage] = time.perf_counter() - started
            if result.returncode:
                raise SystemExit(result.stderr)
            prepared = json.loads(result.stdout)
            snapshot = (Path(cache) / f"{prepared['routing']['key']}.json").read_bytes()
            if stage == 'cold':
                cold_snapshot = snapshot
                (args.output / 'prepared.json').write_text(result.stdout)
                (args.output / 'snapshot.json').write_bytes(snapshot)
            elif snapshot != cold_snapshot:
                raise SystemExit('Warm preparation changed the frozen road snapshot')
            print(f'{stage}: {elapsed[stage]:.6f} s', flush=True)
        legs = len(list(Path(cache).glob('leg-*.json')))
    graph = json.loads(cold_snapshot)
    report = dict(scenario=str(args.scenario.resolve()), router=args.router,
                  workers=args.workers, excluded_public_engineers=excluded,
                  points=len(graph['points']), directed_requests=legs,
                  seconds=elapsed,
                  snapshot_sha256=hashlib.sha256(cold_snapshot).hexdigest())
    if args.reference:
        reference = json.loads(args.reference.read_text())
        report['identical_snapshot'] = reference['snapshot_sha256'] == report['snapshot_sha256']
        if not report['identical_snapshot']:
            (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
            raise SystemExit('REGRESSION: road coverage, costs or geometry changed')
        report['cold_speedup'] = reference['seconds']['cold'] / elapsed['cold']
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
