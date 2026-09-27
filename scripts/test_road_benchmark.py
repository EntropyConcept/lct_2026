#!/usr/bin/env python3
"""Small CLI/isolation/resume checks. Run in the benchmark Python environment."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
BASE=ROOT/'reports/road-benchmark'
BINARY=ROOT/'target/release/dispatch-sat'

def command(*args,env):
    return subprocess.run([str(BINARY),*map(str,args)],env=env,text=True,capture_output=True,cwd=ROOT)

with tempfile.TemporaryDirectory() as temp:
    temp=Path(temp)
    cache=temp/'cache'
    env=dict(os.environ,DISPATCH_ROUTING_CACHE=str(cache))
    no_cache=dict(os.environ);no_cache.pop('DISPATCH_ROUTING_CACHE',None)
    scenario=BASE/'inputs/east-unprepared.json'
    matrix=BASE/'provider/east-normalized.json'
    result=command('benchmark-matrix',scenario,matrix,env=no_cache)
    assert result.returncode and 'explicit isolated' in result.stderr
    result=command('benchmark-matrix',scenario,matrix,env=env)
    assert result.returncode==0,result.stderr
    prepared=temp/'prepared.json';prepared.write_text(result.stdout)
    assert json.loads(result.stdout)['routing']['key']==json.loads((BASE/'inputs/east.json').read_text())['routing']['key']
    reference=json.loads((BASE/'runs/east-sat-fast-1.json').read_text())
    orders=temp/'orders.json';orders.write_text(json.dumps(reference['orders']))
    result=command('check-orders',prepared,orders,env=env)
    assert result.returncode==0 and json.loads(result.stdout)['metrics']==reference['metrics']
    for bad in [reference['orders'][:-1],[[999]]+[[] for _ in range(9)],[[0,0]]+[[] for _ in range(9)]]:
        orders.write_text(json.dumps(bad))
        assert command('check-orders',prepared,orders,env=env).returncode
    original=json.loads(matrix.read_text())
    for kind in ['geometry','time','dimensions','public']:
        bad=copy.deepcopy(original)
        if kind=='geometry':bad['profiles'][0]['legs'][0][0]['shape']='not allowed'
        elif kind=='time':bad['profiles'][0]['legs'][0][0]['minutes']=100001
        elif kind=='dimensions':bad['profiles'][0]['legs'][0].pop()
        else:bad['profiles'][0]['transport']='public'
        badpath=temp/'bad.json';badpath.write_text(json.dumps(bad))
        assert command('benchmark-matrix',scenario,badpath,env=env).returncode,kind

    spec=importlib.util.spec_from_file_location('roadbench',ROOT/'scripts/road-benchmark.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    module.OUT=temp/'experiment';module.OUT.mkdir()
    (module.OUT/'manifest.json').write_text('[]')
    args=SimpleNamespace(family='sat',datasets=[],budgets=[1],resume=False,child=None,scenario=None,seconds=None)
    with patch.object(module.argparse.ArgumentParser,'parse_args',return_value=args):
        module.main() # Empty dataset selection: metadata/archive only; no solver runs.
    archived=module.OUT/'code/scripts/road-benchmark.py'
    assert archived.read_bytes()==(ROOT/'scripts/road-benchmark.py').read_bytes()
    metadata=module.OUT/'environment.json'
    with patch.object(module.argparse.ArgumentParser,'parse_args',return_value=SimpleNamespace(**{**vars(args),'resume':True})):
        module.main() # Unchanged environment is accepted.
        changed=json.loads(metadata.read_text());changed['python']='different environment'
        before=json.dumps(changed);metadata.write_text(before)
        try:
            module.main()
        except RuntimeError as error:
            assert 'Resume environment/code changed' in str(error)
        else:
            raise AssertionError('Changed environment was silently accepted')
        assert metadata.read_text()==before,'Rejected resume overwrote historical metadata'
print('PASS: native replay, invalid indices/duplicates/matrices, isolated cache, executed-code archive and fail-closed resume environment.')
