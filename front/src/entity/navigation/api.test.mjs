import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { atMinute, fromScenario, toScenario, fromResult, minutes } from './api.ts';
import { decodePolyline6, engineerPosition, routeLegs, pointAlong } from './route-view.ts';
const day = new Date(2026, 8, 29).toISOString();
const demo = JSON.parse(await readFile(new URL('../../../../demo.json', import.meta.url), 'utf8'));

test('API adapter preserves minute times, metadata, profiles and midnight', () => {
  const source = { ...demo, notes: ['Keep me'], transit_date: '2026-09-29', routing: { key: 'cached', source: 'OSM' } };
  source.jobs[0].window_end = 1440;
  source.jobs[0].geocode_match = 'Matched address';
  source.engineers[0].already_used = true;
  const restored = toScenario(fromScenario(source, day), day);
  assert.equal(restored.jobs[0].window_end, 1440);
  assert.equal(restored.jobs[0].geocode_match, 'Matched address');
  assert.equal(restored.engineers[0].already_used, true);
  assert.deepEqual(restored.notes, source.notes);
  assert.deepEqual(restored.routing, source.routing);
  assert.equal(restored.transit_date, source.transit_date);
  for (let i = 0; i < source.jobs.length; i++) assert.equal(restored.jobs[i].window_start, source.jobs[i].window_start);
  assert.equal(minutes(atMinute(1440, day), day), 1440);
});

test('solver result maps assignments, distance, baseline and event time', () => {
  const plan = { routes: [{ engineer_id: demo.engineers[0].id, distance_m: 1200, stops: [{ job_id: demo.jobs[0].id, departure: 480, arrival: 500, start: 510, end: 550, explanation: 'Fits', distance_m: 1200 }] }], unassigned: [{ job_id: 'missing', reason: 'No skill' }], metrics: { unassigned: 1, urgent_unassigned: 0, engineers: 1, distance_m: 1200 } };
  const result = fromResult({ scenario: demo, plan, baseline: plan, stats: { optimal: false }, changes: ['Added'], last_event_time: 720 }, day);
  assert.equal(result.plan.routes[0].engineerId, demo.engineers[0].id);
  assert.equal(result.plan.routes[0].stops[0].start.getHours(), 8);
  assert.equal(result.plan.routes[0].stops[0].start.getMinutes(), 30);
  assert.equal(result.plan.metrics.distance, 1200);
  assert.equal(result.plan.unassigned[0].reason, 'No skill');
  assert.deepEqual(result.baseline, result.plan);
  assert.equal(result.lastEventTime.getHours(), 12);
});

test('polyline6 and live travel follow geometry; city legs never invent connectors', () => {
  const encode = points => {
    let previous = [0, 0], value = '';
    for (const [lon, lat] of points) {
      const current = [Math.round(lat * 1e6), Math.round(lon * 1e6)];
      current.forEach((part, i) => { let n = part - previous[i]; n = n < 0 ? ~(n << 1) : n << 1; while (n >= 32) { value += String.fromCharCode((32 | (n & 31)) + 63); n >>>= 5; } value += String.fromCharCode(n + 63); });
      previous = current;
    }
    return value;
  };
  const geometry = [[37.6, 55.7], [37.61, 55.705], [37.62, 55.71]];
  assert.deepEqual(decodePolyline6(encode(geometry)), geometry);
  assert.deepEqual(decodePolyline6('~~~'), []);
  assert.deepEqual(pointAlong(geometry, 0), geometry[0]);
  assert.deepEqual(pointAlong(geometry, 1), geometry.at(-1));
  const scenario = fromScenario(demo, day);
  const engineer = scenario.engineers[0];
  const route = { engineerId: engineer.id, distance: 1000, stops: [{ jobId: scenario.jobs[0].id, departure: atMinute(480, day), arrival: atMinute(500, day), start: atMinute(510, day), end: atMinute(550, day) }] };
  assert.equal(engineerPosition(scenario, engineer, route, atMinute(490, day)).status, 'В пути');
  assert.equal(engineerPosition(scenario, engineer, route, atMinute(505, day)).status, 'Ожидает');
  assert.equal(engineerPosition(scenario, engineer, route, atMinute(520, day)).status, 'На месте');
  assert.equal(routeLegs(scenario, route)[0].length, 2);
  scenario.routing = { key: 'city' };
  assert.deepEqual(routeLegs(scenario, route), [[]]);
});

test('travelled/remaining split follows bends and meets the engineer exactly', async () => {
  const { splitPath, routeProgress } = await import('./route-view.ts');
  const points = [[37.6,55.7],[37.62,55.7],[37.62,55.72]];
  for (const progress of [0, .1, .49, .67, .99, 1]) {
    const split = splitPath(points, progress);
    if (progress > 0 && progress < 1) {
      const position = pointAlong(points, progress);
      assert.deepEqual(split.travelled.at(-1), position);
      assert.deepEqual(split.remaining[0], position);
      assert.ok(split.travelled.length >= 2 && split.remaining.length >= 2);
    }
    if (progress === 0) assert.deepEqual(split.travelled, []);
    if (progress === 1) assert.deepEqual(split.remaining, []);
  }
  const stop = { departure:atMinute(540,day), arrival:atMinute(600,day), start:atMinute(610,day), end:atMinute(640,day) };
  const route = { stops:[stop] };
  assert.equal(routeProgress(route, [points], atMinute(530,day)).percent, 0);
  assert.equal(routeProgress(route, [points], atMinute(580,day)).percent, 67);
  assert.equal(routeProgress(route, [points], atMinute(630,day)).percent, 100);
  assert.equal(routeProgress(route, [[]], atMinute(580,day)).percent, null, 'missing city geometry must not invent movement');
});

test('route progress stays still during service and resumes on the next travel leg', async () => {
  const { routeProgress } = await import('./route-view.ts');
  const points = [[37.6,55.7],[37.62,55.7]];
  const stop = (departure,arrival,start,end) => ({ departure:atMinute(departure,day),arrival:atMinute(arrival,day),start:atMinute(start,day),end:atMinute(end,day) });
  const route = { stops:[stop(540,560,570,600),stop(620,640,640,670)] };
  for (const time of [560,580,610,620]) assert.equal(routeProgress(route,[points,points],atMinute(time,day)).percent,50);
  assert.equal(routeProgress(route,[points,points],atMinute(630,day)).percent,75);
  assert.equal(routeProgress(route,[points,points],atMinute(540,day)).percent,0, 'backward seeking restores the future path');
});
