#!/usr/bin/env python3
"""Offline HTTP integration: road/transit snapshots, cache, events and geocoding.
Run after cargo build --release --locked. No third-party services are called.
"""
import copy
from contextlib import contextmanager
import json
import os
import socket
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen
from urllib.parse import parse_qs, urlparse


def polyline(points):
    result, previous = '', [0, 0]
    for point in points:
        for axis, key in enumerate(('lat', 'lon')):
            value = round(point[key] * 1e6)
            delta = value - previous[axis]
            previous[axis] = value
            encoded = ~(delta << 1) if delta < 0 else delta << 1
            while encoded >= 32:
                result += chr((encoded & 31) + 95)
                encoded >>= 5
            result += chr(encoded + 63)
    return result


calls, version, fail = [], 1, False
transit_calls, transit_fail, transit_walk_only = [], False, False
transit_responses = {}
transit_gate = threading.Barrier(3)
transit_lock = threading.Lock()
transit_dense = False
street_calls, street_metadata_calls = [], []
street_enabled, street_malformed = True, False
street_lock = threading.Lock()


def street_leg(mode, a, b):
    reverse = a['lat'] > b['lat']
    bend = {'lat': (a['lat'] + b['lat']) / 2 + (-.001 if reverse else .001),
            'lon': (a['lon'] + b['lon']) / 2}
    return {'mode': mode,
            'duration': {'CAR': 125, 'WALK': 601, 'BIKE': 245}[mode] + (61 if reverse else 0),
            'distance': 1250 if reverse else 900,
            'steps': [{'osmWay': 123}],
            'legGeometry': {'points': polyline([a, bend, b]), 'length': 3}}


class Provider(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, data, status=200):
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps(data).encode())

    def do_GET(self):
        if self.path == '/api/v1/map/initial':
            street_metadata_calls.append(self.path)
            self.reply({'serverConfig': {'hasStreetRouting': street_enabled,
                                         'maxDirectTimeLimit': 21600}})
            return
        if self.path.startswith('/api/v6/plan?'):
            params = parse_qs(urlparse(self.path).query, keep_blank_values=True)
            if params.get('directModes', [''])[0]:
                with street_lock:
                    street_calls.append(params)
                assert params['transitModes'] == ['']
                assert params['detailedLegs'] == ['true']
                assert params['maxDirectTime'] == ['21600']
                assert params['maxMatchingDistance'] == ['250']
                a = dict(zip(('lat', 'lon'), map(float, params['fromPlace'][0].split(','))))
                b = dict(zip(('lat', 'lon'), map(float, params['toPlace'][0].split(','))))
                assert a != b  # Diagonals must be local, not provider requests.
                leg = street_leg(params['directModes'][0], a, b)
                if street_malformed:
                    leg['legGeometry']['points'] = '_'
                # Minute-truncated timestamps deliberately disagree with duration.
                self.reply({'direct': [{'duration': leg['duration'],
                                        'startTime': '2026-09-27T09:00:00+03:00',
                                        'endTime': '2026-09-27T09:02:00+03:00',
                                        'legs': [leg]}]})
                return
            with transit_lock:
                transit_calls.append(params)
                call_index = len(transit_calls)
            if call_index <= 3:
                transit_gate.wait(timeout=5)
            if transit_fail:
                self.reply({'error': 'timetable unavailable'}, 503)
                return
            a = dict(zip(('lat', 'lon'), map(float, params['fromPlace'][0].split(','))))
            b = dict(zip(('lat', 'lon'), map(float, params['toPlace'][0].split(','))))
            date = params['time'][0][:10]
            points = ([{'lat': a['lat'] + (b['lat'] - a['lat']) * i / 59_999,
                        'lon': a['lon'] + (b['lon'] - a['lon']) * i / 59_999}
                       for i in range(60_000)] if transit_dense else [a, b])
            shape = polyline(points)
            itineraries = []
            for hour in range(9, 23):
                start, end = f'{date}T{hour:02}:00:00+03:00', f'{date}T{hour:02}:20:00+03:00'
                itineraries.append({'startTime': start, 'endTime': end, 'legs': [{
                    'mode': 'WALK' if transit_walk_only else 'BUS',
                    'displayName': 'Тестовый автобус 7', 'distance': 1000,
                    'startTime': start, 'endTime': end,
                    'from': {'name': 'Посадка'}, 'to': {'name': 'Высадка'},
                    'legGeometry': {'points': shape},
                }]})
            payload = {'itineraries': itineraries, 'direct': []}
            if not transit_dense:
                point_key = lambda p: (round(p['lat'] * 1e6), round(p['lon'] * 1e6))
                transit_responses[(date, point_key(a), point_key(b))] = payload
            self.reply(payload)
            return
        self.reply([] if 'unresolved' in self.path else [
            {'lat': '55.752', 'lon': '37.62', 'display_name': 'Verified mock address'}])

    def do_POST(self):
        payload = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        calls.append(payload)
        if fail:
            self.reply({'error': 'upstream unavailable'}, 503)
            return
        assert payload['shape_format'] == 'polyline6'
        assert payload['costing'] in ['auto', 'bicycle', 'pedestrian']
        rows = []
        for i, a in enumerate(payload['sources']):
            row = []
            for j, b in enumerate(payload['targets']):
                same = a == b
                duration = {'auto': 60, 'bicycle': 180, 'pedestrian': 600}[payload['costing']]
                row.append({'from_index': i, 'to_index': j,
                            'time': 0 if same else duration * version,
                            'distance': 0 if same else version * (0.7 if a['lat'] < b['lat'] else 1),
                            'shape': polyline([a, b])})
            rows.append(row)
        self.reply({'units': 'kilometers', 'sources_to_targets': rows})


def api(port, path, data=None, expected=200):
    req = Request(f'http://127.0.0.1:{port}{path}',
                  data=None if data is None else json.dumps(data).encode(),
                  headers={'Content-Type': 'application/json'})
    try:
        with urlopen(req, timeout=30) as response:
            assert response.status == expected
            return json.load(response)
    except HTTPError as error:
        assert error.code == expected, error.read().decode()
        return json.load(error)


@contextmanager
def running_app(env):
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        app_port = sock.getsockname()[1]
    app = subprocess.Popen(['target/release/dispatch-sat', 'serve', str(app_port)], env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                api(app_port, '/api/demo')
                break
            except URLError:
                time.sleep(0.02)
        else:
            raise AssertionError('server did not start')
        yield app_port
    finally:
        app.terminate()
        app.wait(timeout=5)


provider = ThreadingHTTPServer(('127.0.0.1', 0), Provider)
threading.Thread(target=provider.serve_forever, daemon=True).start()
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
with tempfile.TemporaryDirectory(prefix='dispatch-routing-test-') as cache:
    env = dict(os.environ, DISPATCH_ROUTING_CACHE=cache,
               DISPATCH_ROAD_BACKEND='valhalla',
               DISPATCH_VALHALLA_URL=f'http://127.0.0.1:{provider.server_port}',
               DISPATCH_MOTIS_URL=f'http://127.0.0.1:{provider.server_port}',
               DISPATCH_NOMINATIM_URL=f'http://127.0.0.1:{provider.server_port}',
               DISPATCH_TRANSIT_WORKERS='3')
    process = subprocess.Popen(['target/release/dispatch-sat', 'serve', str(port)], env=env,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                source = api(port, '/api/demo')
                break
            except URLError:
                time.sleep(0.02)
        else:
            raise AssertionError('server did not start')
        try:
            urlopen(Request(f'http://127.0.0.1:{port}/api/norms', headers={'Origin': 'https://foreign.example'}), timeout=5)
            raise AssertionError('foreign origin was accepted')
        except HTTPError as e:
            assert e.code == 403
        norms = api(port, '/api/norms')
        assert {n['key']: n['service'] for n in norms} == {
            'connection': 70, 'emergency': 80, 'equipment': 20, 'local': 30}
        source['jobs'] = source['jobs'][:3]
        source['engineers'] = source['engineers'][:2]
        for j in source['jobs']:
            j.update(skill='local', duration=30, window_start=600,
                     window_end=1100, transport='car', urgent=False)
        for e, mode in zip(source['engineers'], ['car', 'bicycle']):
            e.update(transport=mode, skills=['local'], shift_start=540, shift_end=1200)
        api(port, '/api/routing', {'scenario': source}, 400)
        assert not calls
        public = copy.deepcopy(source)
        public['engineers'][0]['transport'] = 'public'
        api(port, '/api/routing', {'scenario': public, 'confirm_coordinates': True}, 400)
        assert not calls
        routed = api(port, '/api/routing', {'scenario': source, 'confirm_coordinates': True})
        assert len(calls) == 2
        # Remove an inactive profile, then later expand at a NEW urgent coordinate.
        routed['engineers'] = routed['engineers'][:1]
        routed = api(port, '/api/routing', {'scenario': routed, 'confirm_coordinates': True})
        assert len(calls) == 2
        first = api(port, '/api/plan', {'scenario': routed, 'seconds': 1, 'mode': 'exact'})
        assert first['stats']['optimal'] and first['plan']['metrics']['unassigned'] == 0
        assert all(s['shape'] for r in first['plan']['routes'] for s in r['stops'])
        compact = copy.deepcopy(first['plan'])
        for r in compact['routes']:
            for stop in r['stops']:
                del stop['shape']
        event_time = 601
        frozen = [s for r in first['plan']['routes'] for s in r['stops'] if s['departure'] < event_time]
        assert frozen
        urgent = dict(source['jobs'][0], id='URG-HTTP', point={'lat': 55.76, 'lon': 37.635},
                      duration=20, window_start=event_time, window_end=1100, urgent=False)
        version = 2  # If old arcs were refetched, published history would no longer validate.
        added = api(port, '/api/plan', {'scenario': routed, 'seconds': 1, 'mode': 'exact',
                    'previous': compact, 'last_event_time': 0,
                    'event': {'kind': 'add_urgent', 'time': event_time, 'job': urgent}})
        assert len(calls) == 4, len(calls)  # New column and row only; no inactive bicycle calls.
        assert added['plan']['metrics']['urgent_unassigned'] == 0
        assert next(j for j in added['scenario']['jobs'] if j['id'] == urgent['id'])['urgent']
        published = {s['job_id']: s for r in added['plan']['routes'] for s in r['stops']}
        assert all(published[s['job_id']] == s for s in frozen)
        assert added['scenario']['routing']['key'] != routed['routing']['key']
        api(port, '/api/plan', {'scenario': added['scenario'], 'previous': added['plan'],
            'event': {'kind': 'add_urgent', 'time': event_time, 'job': urgent},
            'last_event_time': event_time}, 400)
        cancelled = api(port, '/api/plan', {'scenario': added['scenario'], 'seconds': 1,
            'previous': added['plan'], 'last_event_time': event_time,
            'event': {'kind': 'cancel', 'time': event_time, 'job_id': urgent['id']}})
        assert len(cancelled['scenario']['jobs']) == 3 and len(calls) == 4
        geoinput = copy.deepcopy(source)
        geoinput['jobs'] = geoinput['jobs'][:2]
        geoinput['jobs'][1]['address'] = 'unresolved'
        found = api(port, '/api/geocode', {'scenario': geoinput})
        assert found['routing'] is None
        assert found['jobs'][0]['geocode_match'] == 'Verified mock address'
        assert 'НЕ НАЙДЕНО' in found['jobs'][1]['geocode_match']
        assert found['jobs'][1]['point'] == geoinput['jobs'][1]['point']
        fail = True
        failed = copy.deepcopy(source)
        failed['jobs'][0]['point']['lat'] += 0.01
        api(port, '/api/routing', {'scenario': failed, 'confirm_coordinates': True}, 400)
        # Ready at 09:30 cannot catch the 09:00 bus again. A static 20-minute
        # matrix would incorrectly fit both visits before 09:50.
        public = copy.deepcopy(source)
        public['transit_date'] = '2026-09-27'
        public['jobs'] = public['jobs'][:2]
        public['engineers'] = public['engineers'][:1]
        public['engineers'][0]['transport'] = 'public'
        for job in public['jobs']:
            job.update(transport='public', duration=10, window_start=540, window_end=590)
        public = api(port, '/api/routing', {'scenario': public, 'confirm_coordinates': True})
        prepared_calls = len(transit_calls)
        assert prepared_calls == 4  # base→jobs and both directed job→job pairs
        for mode in ['fast', 'exact']:
            missed = api(port, '/api/plan', {'scenario': public, 'seconds': 1, 'mode': mode})
            assert missed['plan']['metrics']['unassigned'] == 1
            assert missed['stats']['optimal']
        assert all(int(query['searchWindow'][0]) < 3600 for query in transit_calls)
        # Existing full-day responses can supply widened coverage while offline.
        for file in Path(cache).glob('transit-v3-*.json'):
            entry = json.loads(file.read_text())
            _, date, origin, destination = json.loads(entry['identity'].split('/', 1)[1].rsplit('/', 2)[0])
            legacy = file.with_name(f"transit-{file.stem.split('-')[2]}.json")
            legacy.write_text(json.dumps(transit_responses[(date, tuple(origin), tuple(destination))]))
            os.utime(legacy, (entry['fetched_at'], entry['fetched_at']))
            file.unlink()
        for job in public['jobs']:
            job['window_end'] = 660
        api(port, '/api/plan', {'scenario': public, 'seconds': 1}, 400)
        transit_fail = True
        public = api(port, '/api/routing', {'scenario': public, 'confirm_coordinates': True})
        assert len(transit_calls) == prepared_calls
        planned = api(port, '/api/plan', {'scenario': public, 'seconds': 1, 'mode': 'exact'})
        stops = planned['plan']['routes'][0]['stops']
        assert [s['arrival'] for s in stops] == [560, 620]
        assert all(s['shape'] and 'BUS' in s['explanation'] for s in stops)
        offline = copy.deepcopy(public)
        offline['routing'] = None
        migrated = api(port, '/api/routing', {'scenario': offline, 'confirm_coordinates': True})
        assert len(transit_calls) == prepared_calls
        replay = api(port, '/api/plan', {'scenario': migrated, 'seconds': 1, 'mode': 'exact'})
        assert replay['plan']['routes'][0]['stops'] == stops
        # The second preparation reads compact pair entries, still offline.
        cached = api(port, '/api/routing', {'scenario': offline, 'confirm_coordinates': True})
        replay = api(port, '/api/plan', {'scenario': cached, 'seconds': 1, 'mode': 'exact'})
        assert replay['plan']['routes'][0]['stops'] == stops
        assert len(transit_calls) == prepared_calls
        stale = copy.deepcopy(public)
        stale['transit_date'] = '2026-09-28'
        api(port, '/api/plan', {'scenario': stale, 'seconds': 1}, 400)
        urgent_public = dict(public['jobs'][0], id='TRANSIT-URGENT', window_start=565, window_end=800,
                             point={'lat': 55.77, 'lon': 37.65})
        transit_fail = False
        changed = api(port, '/api/plan', {'scenario': public, 'seconds': 1, 'mode': 'exact',
                      'previous': planned['plan'], 'event': {'kind': 'add_urgent', 'time': 565, 'job': urgent_public}})
        assert changed['plan']['metrics']['urgent_unassigned'] == 0
        assert changed['plan']['routes'][0]['stops'][0] == stops[0]
        assert len(transit_calls) == prepared_calls + 5
        rejected = copy.deepcopy(public)
        rejected['routing'] = None
        rejected['transit_date'] = '2026-09-29'
        transit_walk_only = True
        api(port, '/api/routing', {'scenario': rejected, 'confirm_coordinates': True}, 400)
        # 25 pairs × 14 departures × ~120 KB formerly exceeded 32 MB.
        # Timetable coverage and selected map paths survive geometry sharing.
        transit_walk_only = False
        transit_dense = True
        dense = copy.deepcopy(public)
        dense['routing'] = None
        dense['transit_date'] = '2026-09-30'
        dense['engineers'][0]['shift_end'] = 1440
        dense['jobs'] = [dict(public['jobs'][0], id=f'DENSE-{i}', window_end=1380,
                              point={'lat': 55.751 + i * .001, 'lon': 37.611 + i * .001})
                         for i in range(5)]
        large = api(port, '/api/routing', {'scenario': dense, 'confirm_coordinates': True})
        transit_fail = True
        large['routing'] = None
        large = api(port, '/api/routing', {'scenario': large, 'confirm_coordinates': True})
        large_plan = api(port, '/api/plan', {'scenario': large, 'seconds': 2, 'mode': 'exact'})
        large_stops = large_plan['plan']['routes'][0]['stops']
        assert large_plan['plan']['metrics']['unassigned'] == 0
        assert [s['arrival'] for s in large_stops] == [560, 620, 680, 740, 800]
        assert all(len(s['shape']) >= 120_000 for s in large_stops)
        # Seed a separate cache with Valhalla at the SAME origin as MOTIS.
        # Selecting MOTIS must neither reuse those costs nor contact Valhalla.
        with tempfile.TemporaryDirectory(prefix='dispatch-street-test-') as street_cache:
            local = copy.deepcopy(source)
            local['routing'] = None
            local.pop('transit_date', None)
            local['jobs'] = [dict(source['jobs'][0], id=f'STREET-{i}', transport=None,
                                 duration=10, window_start=540, window_end=1100, point=point)
                             for i, point in enumerate([
                                 {'lat': 55.75, 'lon': 37.61}, {'lat': 55.76, 'lon': 37.62}])]
            local['engineers'] = [dict(source['engineers'][0], id=f'STREET-{mode}',
                                      transport=mode, start=local['jobs'][0]['point'])
                                  for mode in ['car', 'walk', 'bicycle']]
            local_env = dict(env, DISPATCH_ROUTING_CACHE=street_cache)
            fail, version = False, 1
            with running_app(local_env) as local_port:
                api(local_port, '/api/routing', {'scenario': local, 'confirm_coordinates': True})
            valhalla_calls, timetable_calls = len(calls), len(transit_calls)
            local_env.update(DISPATCH_ROAD_BACKEND='motis',
                             DISPATCH_VALHALLA_URL='http://127.0.0.1:1')
            with running_app(local_env) as local_port:
                streets = api(local_port, '/api/routing',
                              {'scenario': local, 'confirm_coordinates': True})
                assert len(street_metadata_calls) == 1
                assert len(street_calls) == 6  # Two directed arcs for each street profile.
                assert {q['directModes'][0] for q in street_calls} == {'CAR', 'WALK', 'BIKE'}
                assert len(calls) == valhalla_calls and len(transit_calls) == timetable_calls
                street_plans = []
                for engineer, mode in zip(local['engineers'], ['CAR', 'WALK', 'BIKE']):
                    for origin, destination in [(0, 1), (1, 0)]:
                        single = copy.deepcopy(streets)
                        a, b = [local['jobs'][i]['point'] for i in (origin, destination)]
                        single['engineers'] = [dict(engineer, start=a)]
                        single['jobs'] = [dict(local['jobs'][destination],
                                               transport=engineer['transport'])]
                        planned_street = api(local_port, '/api/plan',
                                             {'scenario': single, 'seconds': 1, 'mode': 'exact'})
                        assert planned_street['plan']['metrics']['unassigned'] == 0
                        stop = planned_street['plan']['routes'][0]['stops'][0]
                        expected_leg = street_leg(mode, a, b)
                        assert stop['arrival'] - stop['departure'] == (expected_leg['duration'] + 59) // 60
                        assert stop['distance_m'] == expected_leg['distance']
                        assert stop['shape'] == expected_leg['legGeometry']['points']
                        street_plans.append((single, stop))
                # Provider corruption is an error, not a successful unreachable matrix.
                broken = copy.deepcopy(local)
                broken['engineers'] = broken['engineers'][:1]
                broken['jobs'][1]['point']['lat'] += .02
                street_malformed = True
                api(local_port, '/api/routing', {'scenario': broken, 'confirm_coordinates': True}, 400)
                street_malformed = False
                attempted_arcs = len(street_calls)
                street_enabled = False
                api(local_port, '/api/routing', {'scenario': broken, 'confirm_coordinates': True}, 400)
                assert len(street_calls) == attempted_arcs
                street_enabled = True
            # Close the actual router socket, then restart the application: only
            # fresh on-disk directed leg entries can reconstruct this snapshot.
            provider.shutdown()
            provider.server_close()
            offline_calls = (len(street_metadata_calls), len(street_calls), len(calls))
            with running_app(local_env) as local_port:
                restored = api(local_port, '/api/routing',
                               {'scenario': local, 'confirm_coordinates': True})
                assert restored['routing']['key'] == streets['routing']['key']
                for single, stop in street_plans:
                    single['routing'] = restored['routing']
                    replay = api(local_port, '/api/plan',
                                 {'scenario': single, 'seconds': 1, 'mode': 'exact'})
                    assert replay['plan']['routes'][0]['stops'] == [stop]
                extended = copy.deepcopy(restored)
                extended['jobs'].append(dict(local['jobs'][0], id='STREET-UNCACHED',
                                             point={'lat': 55.79, 'lon': 37.66}))
                api(local_port, '/api/routing', {'scenario': extended, 'confirm_coordinates': True}, 400)
                assert (len(street_metadata_calls), len(street_calls), len(calls)) == offline_calls
            assert len(calls) == valhalla_calls and len(transit_calls) == timetable_calls
        print('PASS: road cache, concurrent transit requests, legacy/compact offline replay, >32 MB repeated geometry, departures, urgent events, immutable history, local MOTIS CAR/WALK/BIKE, backend cache separation, directed geometry/costs, offline restart and closed provider failures')
    finally:
        process.terminate()
        process.wait(timeout=5)
        provider.shutdown()
