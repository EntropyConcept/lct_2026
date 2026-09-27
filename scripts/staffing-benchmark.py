#!/usr/bin/env python3
"""Cold-start exact-mode quality check against an exported day-start plan.
Only scenario is passed to the solver: the reference plan is NOT a warm start.
Requires the original city routing cache. Timing/quality depend on CPU load.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('plan', type=Path)
parser.add_argument('--seconds', type=float, default=30)
parser.add_argument('--runs', type=int, default=3)
parser.add_argument('--binary', default='target/release/dispatch-sat')
parser.add_argument('--output', type=Path)
args = parser.parse_args()
if args.runs < 1:
    parser.error('--runs must be positive')
reference = json.loads(args.plan.read_text())
if reference.get('last_event_time', 0) or reference.get('changes'):
    parser.error('Use a day-start export without dispatched event history')
keys = ('urgent_unassigned', 'unassigned', 'engineers')
target = tuple(reference['plan']['metrics'][key] for key in keys)
results = []
with tempfile.TemporaryDirectory() as directory:
    scenario = Path(directory) / 'scenario.json'
    scenario.write_text(json.dumps(reference['scenario']))
    for run in range(args.runs):
        started = time.perf_counter()
        result = json.loads(subprocess.check_output([
            args.binary, 'solve', str(scenario), str(args.seconds), 'exact'
        ], timeout=max(30, args.seconds + 30)))
        stats = result['stats']
        row = {'run': run + 1, 'metrics': result['plan']['metrics'],
               'elapsed_ms': stats['elapsed_ms'], 'wall_ms': round((time.perf_counter() - started) * 1000),
               'stages': stats['stages'],
               'global_optimal': stats['optimal']}
        results.append(row)
        if args.output:
            args.output.write_text(json.dumps(results, indent=2) + '\n')
        print(json.dumps(row), flush=True)
        assert stats['scope'] == 'global'
        assert tuple(row['metrics'][key] for key in keys) <= target, 'Staffing/coverage target missed'
