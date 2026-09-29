import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { format, startOfDay } from 'date-fns';
import { navigationFactory } from './model.ts';
const demo = JSON.parse(await readFile(new URL('../../../../demo.json', import.meta.url), 'utf8'));
const today = () => format(new Date(), 'yyyy-MM-dd');
const setup = handler => {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (path, options) => {
    const body = options?.body && JSON.parse(options.body);
    calls.push({ path, body });
    const value = await handler(path, body);
    return new Response(JSON.stringify(value), { status: value?.error ? 503 : 200 });
  };
  return { calls, restore: () => { globalThis.fetch = original; } };
};

test('today and exact SAT / thirty seconds are defaults; dataset loading prepares for selected date', async () => {
  const nav = navigationFactory();
  assert.equal(nav.$date.get(), startOfDay(new Date()).toISOString());
  assert.equal(nav.$scenario.get().value.transit_date, today());
  assert.equal(nav.$solver.get(), 'exact');
  assert.equal(nav.$seconds.get(), 30);
  const mock = setup((path, body) => path === '/api/demo'
    ? { ...demo, transit_date: '2020-01-01', routing: { key: 'obsolete' } }
    : { ...body.scenario, routing: { key: 'prepared' } });
  try {
    assert.equal(await nav.load('/api/demo'), true);
    assert.equal(mock.calls[1].path, '/api/routing');
    assert.equal(mock.calls[1].body.scenario.transit_date, today());
    assert.equal(mock.calls[1].body.scenario.routing, undefined);
    assert.equal(mock.calls[1].body.confirm_coordinates, true);
    assert.equal(nav.$scenario.get().value.routing.key, 'prepared');
    assert.equal(nav.$needsRouting.get(), false);
    nav.setDate(new Date(2026, 9, 3));
    await nav.load('/api/demo');
    assert.equal(mock.calls.at(-1).body.scenario.transit_date, '2026-10-03');
  } finally { mock.restore(); }
});

test('failed automatic preparation retains data, blocks solve, and can be retried', async () => {
  const nav = navigationFactory();
  let fail = true;
  const mock = setup((path, body) => path === '/api/demo' ? demo : fail ? { error: 'Router unavailable' } : { ...body.scenario, routing: { key: 'ready' } });
  try {
    assert.equal(await nav.load('/api/demo'), false);
    assert.equal(nav.$scenario.get().value.jobs.length, demo.jobs.length);
    assert.equal(nav.$needsRouting.get(), true);
    assert.equal(nav.$busy.get(), false);
    assert.equal(nav.$status.get().value, 'initial');
    assert.equal(await nav.startSearch(), false);
    assert.equal(mock.calls.some(call => call.path === '/api/plan'), false);
    fail = false;
    assert.equal(await nav.prepare(true), true);
    assert.equal(nav.$needsRouting.get(), false);
    assert.equal(nav.$error.get(), '');
  } finally { mock.restore(); }
});

test('multi-day import preserves explicit dates and prepares every bucket even after a routing failure', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => path === '/api/import' ? JSON.parse(body.text)
    : body.scenario.transit_date === '2026-10-02' ? { error: 'First day unavailable' }
    : { ...body.scenario, routing: { key: 'second-day' } });
  try {
    const days = ['2026-10-02', '2026-10-03'].map(date => ({ date, scenario: { ...demo, transit_date: '2020-01-01', routing: { key: 'old' } } }));
    assert.equal(await nav.importFile('days.json', JSON.stringify({ days })), false);
    assert.equal(format(new Date(nav.$date.get()), 'yyyy-MM-dd'), '2026-10-02');
    assert.deepEqual(mock.calls.filter(call => call.path === '/api/routing').map(call => call.body.scenario.transit_date), ['2026-10-02', '2026-10-03']);
    assert.equal(nav.$needsRouting.get(), true);
    nav.setDate(new Date(2026, 9, 3));
    assert.equal(nav.$scenario.get().value.routing.key, 'second-day');
    assert.equal(nav.$needsRouting.get(), false);
  } finally { mock.restore(); }
});

test('invalid multi-day import remains transactional and does not start preparation', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => JSON.parse(body.text));
  try {
    const days = [{ date: '2026-10-02', scenario: demo }, { date: 'invalid', scenario: demo }];
    assert.equal(await nav.importFile('days.json', JSON.stringify({ days })), false);
    assert.equal(nav.$scenario.get().value.jobs.length, 0);
    assert.equal(mock.calls.some(call => call.path === '/api/routing'), false);
  } finally { mock.restore(); }
});

test('undated CSV import uses current day and automatically prepares roads', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => path === '/api/import' ? { ...demo, transit_date: undefined } : { ...body.scenario, routing: { key: 'csv' } });
  try {
    assert.equal(await nav.importFile('data.csv', 'fixture'), true);
    assert.equal(mock.calls.at(-1).body.scenario.transit_date, today());
    assert.equal(nav.$scenario.get().value.routing.key, 'csv');
  } finally { mock.restore(); }
});

test('manual seeking resets fractional playback even within the same minute', () => {
  const nav = navigationFactory();
  nav.$playhead.set(720.67);
  nav.seekTime(720);
  assert.equal(nav.$playhead.get(),720);
  nav.$playhead.set(720.5);
  nav.setDate(new Date(2026,9,3));
  assert.equal(nav.$playhead.get(), nav.$time.get());
});

const solution = (scenario, metrics = {}, optimal = false) => {
  const plan = { routes: [], unassigned: [], metrics: { urgent_unassigned: 0, unassigned: 0, engineers: 2, distance_m: 1000, ...metrics } };
  return { scenario, plan, baseline: plan, stats: { optimal }, changes: [], last_event_time: 0 };
};

test('search publishes intermediate solutions with increasing budgets and keeps the best objective', async () => {
  const nav = navigationFactory();
  let attempt = 0;
  const candidates = [
    { urgent_unassigned: 1, distance_m: 1 },
    { unassigned: 2, distance_m: 3000 },
    { unassigned: 3, distance_m: 1 }, // Worse despite a shorter route.
    { unassigned: 2, engineers: 1, distance_m: 4000 },
    { unassigned: 2, engineers: 1, distance_m: 4000 },
  ];
  const distances = [];
  const unsubscribe = nav.$result.subscribe(result => { if (result.value) distances.push(result.value.plan.metrics.distance); });
  const mock = setup((path, body) => {
    assert.equal(path, '/api/plan');
    assert.equal(nav.$busy.get(), true);
    assert.equal(nav.$searchSeconds.get(), body.seconds);
    return solution(body.scenario, candidates[attempt++]);
  });
  try {
    const pending = nav.startSearch();
    assert.equal(await nav.startSearch(), false, 'overlapping searches are blocked');
    assert.equal(await pending, true);
    assert.deepEqual(mock.calls.map(call => call.body.seconds), [1, 5, 10, 15, 30]);
    assert.deepEqual(distances, [1, 3000, 4000, 4000]);
    assert.equal(nav.$status.get().value, 'suboptimal');
    assert.equal(nav.$busy.get(), false);
    assert.equal(nav.$searchSeconds.get(), null);
  } finally { unsubscribe(); mock.restore(); }
});

test('search stops immediately when optimality is proven', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => solution(body.scenario, {}, body.seconds === 5));
  try {
    assert.equal(await nav.startSearch(), true);
    assert.deepEqual(mock.calls.map(call => call.body.seconds), [1, 5]);
    assert.equal(nav.$status.get().value, 'optimal');
    assert.equal(nav.$searchSeconds.get(), null);
  } finally { mock.restore(); }
});

test('search respects custom maximum budgets and rejects invalid budgets before requesting', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => solution(body.scenario));
  try {
    for (const [limit, expected] of [[0.5, [0.5]], [5, [1, 5]], [12, [1, 5, 10, 12]], [40, [1, 5, 10, 15, 30, 40]]]) {
      mock.calls.length = 0;
      nav.$seconds.set(limit);
      assert.equal(await nav.startSearch(), true);
      assert.deepEqual(mock.calls.map(call => call.body.seconds), expected);
    }
    for (const limit of [0, NaN, Infinity, 301]) {
      mock.calls.length = 0;
      nav.$seconds.set(limit);
      assert.equal(await nav.startSearch(), false);
      assert.equal(mock.calls.length, 0);
    }
  } finally { mock.restore(); }
});

test('failed refinement retains the published result and releases the UI', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => body.seconds === 1 ? solution(body.scenario) : { error: 'Solver unavailable' });
  try {
    assert.equal(await nav.startSearch(), false);
    assert.deepEqual(mock.calls.map(call => call.body.seconds), [1, 5]);
    assert.equal(nav.$result.get().value.plan.metrics.distance, 1000);
    assert.equal(nav.exportResult().plan.metrics.distance_m, 1000);
    assert.equal(nav.$status.get().value, 'suboptimal');
    assert.equal(nav.$error.get(), 'Solver unavailable');
    assert.equal(nav.$busy.get(), false);
    assert.equal(nav.$searchSeconds.get(), null);
  } finally { mock.restore(); }
});

test('failed first attempt restores the previous status without leaving search active', async () => {
  const nav = navigationFactory();
  const mock = setup(() => ({ error: 'Solver unavailable' }));
  try {
    assert.equal(await nav.startSearch(), false);
    assert.equal(mock.calls.length, 1);
    assert.equal(nav.$result.get().value, null);
    assert.equal(nav.$status.get().value, 'initial');
    assert.equal(nav.$busy.get(), false);
    assert.equal(nav.$searchSeconds.get(), null);
  } finally { mock.restore(); }
});

test('event refinement reuses original scenario and history without applying the event twice', async () => {
  const nav = navigationFactory();
  const mock = setup((path, body) => {
    if (!body.event) return solution(body.scenario, {}, true);
    const result = solution({ ...body.scenario, name: 'After event' });
    return { ...result, changes: ['Cancelled'], last_event_time: body.event.time };
  });
  try {
    await nav.startSearch();
    const original = nav.exportResult();
    assert.equal(await nav.cancelJob('J06'), true);
    const attempts = mock.calls.slice(1).map(call => call.body);
    assert.deepEqual(attempts.map(body => body.seconds), [1, 5, 10, 15, 30]);
    for (const { seconds, ...body } of attempts) {
      assert.deepEqual(body, {
        scenario: original.scenario, mode: 'exact', previous: original.plan,
        event: { kind: 'cancel', time: 720, job_id: 'J06' }, last_event_time: 0,
      });
    }
    assert.equal(nav.$historyLocked.get(), true);
    assert.equal(nav.exportResult().scenario.name, 'After event');
    assert.deepEqual(nav.exportResult().changes, ['Cancelled']);
  } finally { mock.restore(); }
});
