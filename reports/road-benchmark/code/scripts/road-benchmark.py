#!/usr/bin/env python3
"""Sequential, single-thread solver benchmark on frozen real road tables.
Run prepare-road-benchmark.py first. Use a Python environment with pinned OR-Tools.
Each cell is a fresh process; no plan from another solver/budget is reused.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'reports/road-benchmark'
BINARY = ROOT / 'target/release/dispatch-sat'
CACHE = OUT / 'snapshots'
ENV = dict(os.environ, DISPATCH_ROUTING_CACHE=str(CACHE), OMP_NUM_THREADS='1', OPENBLAS_NUM_THREADS='1')
METRICS = ('urgent_unassigned', 'unassigned', 'engineers', 'distance_m')


def key(metrics):
    return tuple(metrics[name] for name in METRICS)


def orders_of(scenario, plan):
    ids = {j['id']: i for i, j in enumerate(scenario['jobs'])}
    routes = {r['engineer_id']: r for r in plan['routes']}
    return [[ids[v['job_id']] for v in routes[e['id']]['stops']] for e in scenario['engineers']]


def load_data(path):
    scenario = json.loads(path.read_text())
    snap = json.loads((CACHE / (scenario['routing']['key']+'.json')).read_text())
    def rounded(v):
        return math.floor(v*1e6+.5) if v>=0 else math.ceil(v*1e6-.5)
    def point(p):
        return rounded(p['lat']), rounded(p['lon'])
    positions = {point(p): i for i, p in enumerate(snap['points'])}
    jobs, engineers = scenario['jobs'], scenario['engineers']
    n, m = len(jobs), len(engineers)
    targets = [positions[point(j['point'])] for j in jobs]
    times, distances, eligible = [], [], []
    for e in engineers:
        profile = next(p for p in snap['profiles'] if p['transport']==e['transport'])
        indices = targets + [positions[point(e['start'])]]
        legs = [[profile['legs'][i][j] for j in targets] for i in indices]
        times.append([[v['minutes'] if v else 100000 for v in row] for row in legs])
        distances.append([[v['metres'] if v else 0 for v in row] for row in legs])
        eligible.append([j['skill'] in e['skills'] and (not j.get('transport') or j['transport']==e['transport']) for j in jobs])
    # Exactly one incoming leg per served job bounds total distance, without a return leg.
    distance_bound = sum(max((distances[e][i][j] for e in range(m) for i in range(n+1)), default=0) for j in range(n))
    staff = distance_bound + 1
    unassigned = (m+1)*staff
    urgent = (n+1)*unassigned
    assert n*(urgent+unassigned)+m*staff+distance_bound < 2**62
    return dict(scenario=scenario,n=n,m=m,minutes=times,metres=distances,eligible=eligible,
                weights=dict(staff=staff,unassigned=unassigned,urgent=urgent))


def run_child(path, method, seconds):
    # Imports before the measured algorithm interval; outer wall time includes them.
    from bench_comparators import solve_routing, solve_cpsat, replay
    started = time.monotonic()
    data = load_data(path)
    seeds = []
    for strategy in ('append','insertion'):
        response = json.loads(subprocess.check_output([str(BINARY),'heuristic',str(path),strategy],cwd=ROOT,env=ENV))
        seeds.append(response['plan'])
    seed = min(seeds,key=lambda p:key(p['metrics']))
    data['seed_orders'] = orders_of(data['scenario'],seed)
    assert replay(data,data['seed_orders'])[0] == key(seed['metrics'])
    result = (solve_routing if method=='routing-gls' else solve_cpsat)(data,started+seconds)
    result['elapsed_ms'] = round((time.monotonic()-started)*1000)
    result['weights'] = data['weights']
    print(json.dumps(result))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--family',choices=['sat','comparators','all'],default='all')
    parser.add_argument('--datasets',nargs='+',default=['east','southeast','southcentre'])
    parser.add_argument('--budgets',nargs='+',type=float,default=[1,5,30,120])
    parser.add_argument('--resume',action='store_true')
    parser.add_argument('--child',choices=['routing-gls','cp-sat'])
    parser.add_argument('--scenario',type=Path)
    parser.add_argument('--seconds',type=float)
    args = parser.parse_args()
    if any(not math.isfinite(s) or not .01 <= s <= 300 for s in args.budgets):
        parser.error('budgets must be finite and within 0.01..300 seconds')
    if args.child and (args.scenario is None or args.seconds is None or not math.isfinite(args.seconds) or not .01 <= args.seconds <= 300):
        parser.error('child requires a scenario and finite seconds within 0.01..300')
    if args.child:
        run_child(args.scenario,args.child,args.seconds)
        return
    manifest = {r['dataset']:r for r in json.loads((OUT/'manifest.json').read_text())}
    sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
    fingerprint = {'binary_sha256':sha(BINARY),'runner_sha256':sha(Path(__file__)),
                   'comparators_sha256':sha(ROOT/'scripts/bench_comparators.py')}
    import ortools
    metadata = dict(platform=platform.platform(),python=sys.version,ortools=ortools.__version__,
                    cpu=subprocess.check_output(['sysctl','-n','machdep.cpu.brand_string'],text=True).strip(),
                    seed=1,workers=1,repetitions=1,fast_budget_s=1,
                    timing='Budget covers matrices/seed/model/search. Wall includes process startup, serialization and teardown; validation excluded.',
                    **fingerprint)
    (OUT/'environment.json').write_text(json.dumps(metadata,indent=2)+'\n')
    (OUT/'runs').mkdir(exist_ok=True)
    methods=[]
    if args.family in ('sat','all'):
        methods += [('greedy-append',None),('greedy-insertion',None),('sat-fast',1)]
        methods += [('sat-exact',s) for s in args.budgets]
    if args.family in ('comparators','all'):
        methods += [(method,s) for method in ('routing-gls','cp-sat') for s in args.budgets]
    for dataset in args.datasets:
        path = OUT/'inputs'/f'{dataset}.json'
        entry = manifest[dataset]
        assert sha(path)==entry['scenario_sha256']
        assert sha(CACHE/(entry['snapshot']+'.json'))==entry['snapshot_sha256']
        scenario=json.loads(path.read_text())
        for method,seconds in methods:
            tag=f'{dataset}-{method}-{seconds:g}' if seconds is not None else f'{dataset}-{method}'
            target=OUT/'runs'/(tag+'.json')
            if args.resume and target.exists():
                old=json.loads(target.read_text())
                assert old['fingerprint']==fingerprint and old['scenario_sha256']==entry['scenario_sha256']
                continue
            if method.startswith('greedy-'):
                command=[str(BINARY),'heuristic',str(path),method.removeprefix('greedy-')]
            elif method.startswith('sat-'):
                command=[str(BINARY),'solve',str(path),str(seconds),method.removeprefix('sat-')]
            else:
                command=[sys.executable,str(Path(__file__).resolve()),'--child',method,'--scenario',str(path),'--seconds',str(seconds)]
            print('START',tag,flush=True)
            start=time.monotonic()
            process=subprocess.run(command,cwd=ROOT,env=ENV,text=True,capture_output=True,timeout=max(90,(seconds or 0)+60))
            wall_ms=round((time.monotonic()-start)*1000)
            (OUT/'runs'/(tag+'.stderr')).write_text(process.stderr)
            process.check_returncode()
            answer=json.loads(process.stdout)
            (OUT/'runs'/(tag+'.raw.json')).write_text(json.dumps(answer,ensure_ascii=False)+'\n')
            orders=answer['orders'] if 'orders' in answer else orders_of(scenario,answer['plan'])
            # Independent native replay, outside solver timing. This also regenerates full metrics.
            with tempfile.TemporaryDirectory() as temp:
                orderfile=Path(temp)/'orders.json';orderfile.write_text(json.dumps(orders))
                checked=json.loads(subprocess.check_output([str(BINARY),'check-orders',str(path),str(orderfile)],cwd=ROOT,env=ENV))
            metrics=checked['metrics']
            claimed=tuple(answer['objective']) if 'objective' in answer else key(answer['plan']['metrics'])
            assert key(metrics)==claimed, (tag,metrics,claimed)
            stats=answer.get('stats',{})
            global_optimal=bool(stats.get('optimal',False) and stats.get('scope')=='global') if stats else bool(answer.get('optimal',False))
            staff_proven=global_optimal or (stats.get('scope')=='global' and any(v['criterion']=='engineers' and v['proven'] for v in stats.get('stages',[])))
            row=dict(dataset=dataset,jobs=len(scenario['jobs']),method=method,budget_s=seconds,metrics=metrics,
                     wall_ms=wall_ms,elapsed_ms=stats.get('elapsed_ms',answer.get('elapsed_ms')),
                     global_optimal=global_optimal,staff_proven=staff_proven,
                     incumbent_origin=answer.get('incumbent_origin','sat_pipeline' if method.startswith('sat-') else 'greedy'),
                     scope=stats.get('scope','global' if method=='cp-sat' else 'heuristic'),
                     status=stats.get('status',answer.get('status','validated greedy plan')),
                     stages=stats.get('stages',[]),validated=True,orders=orders,
                     snapshot=entry['snapshot'],scenario_sha256=entry['scenario_sha256'],fingerprint=fingerprint)
            target.write_text(json.dumps(row,ensure_ascii=False,indent=2)+'\n')
            print('DONE',tag,metrics,'wall_ms',wall_ms,'global_optimal',global_optimal,flush=True)

if __name__=='__main__':
    main()
