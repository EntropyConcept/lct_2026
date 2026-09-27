#!/usr/bin/env python3
"""Freeze OSRM/FOSSGIS car, bike, foot road tables at SYNTHETIC CSV test points.
No geocoding, geometry, traffic or transit claims. Nine table requests total;
cache raw responses, obey one request/second, never use fallback_speed.
"""
import hashlib
import json
import math
import os
import subprocess
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / 'reports/road-benchmark'
BINARY = ROOT / 'target/release/dispatch-sat'
NAMES = {'Восток': 'east', 'Юго-восток': 'southeast', 'Югоцентр': 'southcentre'}
for folder in ['inputs', 'snapshots', 'provider']:
    (OUT / folder).mkdir(parents=True, exist_ok=True)
cache_env = dict(os.environ, DISPATCH_ROUTING_CACHE=str(OUT / 'snapshots'))
manifest = []
last_request = 0
for csv in sorted((ROOT / 'dataset').glob('*Синтетические*.csv')):
    slug = NAMES[csv.name.split()[0]]
    started = time.perf_counter()
    scenario = json.loads(subprocess.check_output([str(BINARY), 'import', str(csv)], cwd=ROOT))
    excluded = [e['id'] for e in scenario['engineers'] if e['transport'] == 'public']
    scenario['engineers'] = [e for e in scenario['engineers'] if e['transport'] != 'public']
    scenario['notes'].append('Benchmark: intentionally SYNTHETIC coordinates, workforce and base. Real OSRM/FOSSGIS road tables, no geometry/traffic. Public engineers explicitly excluded: '+', '.join(excluded))
    points = []
    for p in [j['point'] for j in scenario['jobs']] + [e['start'] for e in scenario['engineers']]:
        p = {k:round(p[k],6) for k in ('lat','lon')}
        if p not in points:
            points.append(p)
    profiles, sources = [], []
    for transport, endpoint in [('car','car'),('bicycle','bike'),('walk','foot')]:
        coords = ';'.join(f'{p["lon"]:.6f},{p["lat"]:.6f}' for p in points)
        url = f'https://routing.openstreetmap.de/routed-{endpoint}/table/v1/driving/{coords}?annotations=duration,distance'
        raw = OUT / 'provider' / f'{slug}-{endpoint}.json'
        info = raw.with_suffix('.meta.json')
        if not raw.exists():
            time.sleep(max(0, last_request + 1.1 - time.monotonic()))
            last_request = time.monotonic()
            request = urllib.request.Request(url, headers={'User-Agent':'dispatch-sat-benchmark/0.1 (three synthetic VRPTW datasets)'})
            with urllib.request.urlopen(request, timeout=120) as response:
                body = response.read(20_000_001)
            assert len(body) <= 20_000_000
            value = json.loads(body)
            assert value.get('code') == 'Ok', value
            raw.write_bytes(body)
            info.write_text(json.dumps({'url':url,'retrieved_utc':datetime.now(timezone.utc).isoformat()},indent=2))
        else:
            assert json.loads(info.read_text())['url'] == url, 'Cached table belongs to different coordinates'
            value = json.loads(raw.read_bytes())
        assert value.get('code') == 'Ok' and not value.get('fallback_speed_cells')
        assert len(value['durations']) == len(points) == len(value['distances'])
        legs = []
        for ts, ds in zip(value['durations'], value['distances'], strict=True):
            assert len(ts) == len(points) == len(ds)
            row = []
            for seconds, metres in zip(ts,ds,strict=True):
                if seconds is None or metres is None:
                    assert seconds is None and metres is None
                    row.append(None)
                else:
                    assert math.isfinite(seconds) and math.isfinite(metres) and 0<=seconds<=6_000_000 and 0<=metres<=20_000_000
                    row.append({'minutes':math.ceil((seconds+(1 if metres>0 else 0))/60), 'metres':math.ceil(metres), 'shape':''})
            legs.append(row)
        profiles.append({'transport':transport,'legs':legs})
        sources.append(dict(profile=transport,raw_sha256=hashlib.sha256(raw.read_bytes()).hexdigest(),**json.loads(info.read_text())))
        print(slug, transport, len(points), 'points; real road table cached',flush=True)
    original = OUT / 'inputs' / f'{slug}-unprepared.json'
    original.write_text(json.dumps(scenario,ensure_ascii=False,indent=2))
    matrix = OUT / 'provider' / f'{slug}-normalized.json'
    matrix.write_text(json.dumps({'points':points,'profiles':profiles},separators=(',',':')))
    prepared = json.loads(subprocess.check_output([str(BINARY),'benchmark-matrix',str(original),str(matrix)],cwd=ROOT,env=cache_env))
    prepared['routing']['source'] = 'OSRM/FOSSGIS car-bike-foot road tables; benchmark only; no geometry'
    output = OUT / 'inputs' / f'{slug}.json'
    output.write_text(json.dumps(prepared,ensure_ascii=False,indent=2)+'\n')
    key = prepared['routing']['key']
    snapshot = OUT / 'snapshots' / f'{key}.json'
    row = dict(dataset=slug,csv=str(csv.relative_to(ROOT)),jobs=len(scenario['jobs']),engineers=len(scenario['engineers']),excluded_engineers=excluded,
               snapshot=key,scenario_sha256=hashlib.sha256(output.read_bytes()).hexdigest(),snapshot_sha256=hashlib.sha256(snapshot.read_bytes()).hexdigest(),
               csv_sha256=hashlib.sha256(csv.read_bytes()).hexdigest(),preparation_wall_s=round(time.perf_counter()-started,3),sources=sources,
               coordinate_provenance='synthetic district-centre jitter, NOT geocoded addresses',workforce_provenance='synthetic imported skills, common base and 09:00-23:59 shifts')
    manifest.append(row)
    (OUT / 'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n')
    print(slug,'READY',key,flush=True)
