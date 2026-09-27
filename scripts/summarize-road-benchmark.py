#!/usr/bin/env python3
"""Audit frozen benchmark artifacts and regenerate CSV/Typst data, without solving."""
import csv
import hashlib
import json
import math
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASE = ROOT / 'reports/road-benchmark'
METRICS = ['urgent_unassigned', 'unassigned', 'engineers', 'distance_m']
DATASETS = ['east', 'southeast', 'southcentre']
METHODS = ['greedy-append','greedy-insertion','sat-fast','sat-exact','routing-gls','cp-sat']
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
score = lambda r: tuple(r['metrics'][name] for name in METRICS)
manifest = json.loads((BASE/'manifest.json').read_text())
environment = json.loads((BASE/'environment.json').read_text())
paths = [p for p in (BASE/'runs').glob('*.json') if '.raw.' not in p.name]
runs = [json.loads(p.read_text()) for p in paths]
assert len(runs) == 45 and all(r['validated'] for r in runs)
expected = {(d,m,t) for d in DATASETS for m in METHODS
            for t in ([None] if m.startswith('greedy-') else [1] if m=='sat-fast' else [1,5,30,120])}
assert {(r['dataset'],r['method'],r['budget_s']) for r in runs} == expected
assert all(all(environment[k]==v for k,v in r['fingerprint'].items()) for r in runs)
assert sha(BASE/'code/scripts/road-benchmark.py') == environment['runner_sha256']
assert sha(BASE/'code/scripts/bench_comparators.py') == environment['comparators_sha256']

for entry in manifest:
    name = entry['dataset']
    assert sha(ROOT/entry['csv']) == entry['csv_sha256']
    assert sha(BASE/'inputs'/f'{name}.json') == entry['scenario_sha256']
    assert sha(BASE/'snapshots'/f"{entry['snapshot']}.json") == entry['snapshot_sha256']
    normalized = json.loads((BASE/'provider'/f'{name}-normalized.json').read_text())
    saved = json.loads((BASE/'snapshots'/f"{entry['snapshot']}.json").read_text())
    assert normalized == saved
    for source in entry['sources']:
        mode = {'car':'car','bicycle':'bike','walk':'foot'}[source['profile']]
        rawpath = BASE/'provider'/f'{name}-{mode}.json'
        assert sha(rawpath) == source['raw_sha256']
        raw = json.loads(rawpath.read_text())
        assert raw['code']=='Ok' and not raw.get('fallback_speed_cells')
        profile = next(p for p in saved['profiles'] if p['transport']==source['profile'])
        for i,row in enumerate(profile['legs']):
            for j,leg in enumerate(row):
                seconds,metres = raw['durations'][i][j],raw['distances'][i][j]
                expected_leg = None if seconds is None else {'minutes':math.ceil((seconds+(1 if metres>0 else 0))/60),'metres':math.ceil(metres),'shape':''}
                assert leg == expected_leg
    related = [r for r in runs if r['dataset']==name]
    assert all(r['scenario_sha256']==entry['scenario_sha256'] and r['snapshot']==entry['snapshot'] for r in related)
    # Cross-check every global proven prefix against ALL methods' validated plans.
    for result in related:
        if result['global_optimal']:
            assert all(score(other)>=score(result) for other in related)
        if result['scope']=='global':
            for stage in result['stages']:
                if stage['proven']:
                    k = METRICS.index(stage['criterion'])
                    assert stage['lower']==stage['upper']==result['metrics'][stage['criterion']]
                    assert all(score(other)[:k+1]>=score(result)[:k+1] for other in related)

runs.sort(key=lambda r:(DATASETS.index(r['dataset']),METHODS.index(r['method']),r['budget_s'] or 0))
fields = ['dataset','method','budget_s',*METRICS,'elapsed_ms','wall_ms','staff_proven','global_optimal','scope','status','incumbent_origin','validated','snapshot']
with (BASE/'results.csv').open('w',newline='') as f:
    writer=csv.DictWriter(f,fieldnames=fields);writer.writeheader()
    for r in runs:
        flat={**r,**r['metrics']};writer.writerow({k:flat[k] for k in fields})
models=[]
for dataset in DATASETS:
    for method,budget in [('sat-fast',1),('sat-exact',120)]:
        raw=json.loads((BASE/'runs'/f'{dataset}-{method}-{budget}.raw.json').read_text())
        models.append(dict(dataset=dataset,method=method,**raw['stats']))
result=dict(date=datetime.fromtimestamp(max(p.stat().st_mtime for p in paths),timezone.utc).strftime('%d.%m.%Y'),
            environment=environment,python_short=environment['python'].split()[0],runs=runs,models=models,
            global_optima=sum(r['global_optimal'] for r in runs),staff_proofs=sum(r['staff_proven'] for r in runs))
(ROOT/'reports/article/results.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
print('PASS: 45 rows, identical execution fingerprints; all input/raw/normalized hashes and matrix cells verified; global proven prefixes consistent across methods.')
print('Wrote reports/road-benchmark/results.csv and reports/article/results.json')
