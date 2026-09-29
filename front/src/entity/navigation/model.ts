import { format, startOfDay } from 'date-fns';
import { atom, computed, map } from 'nanostores';
import type { Scenario, PlanResult } from './types';
import type { Job } from './job';
import { request, fromScenario, toScenario, fromResult, toJob, type ApiScenario, type ApiResult, type Norm } from './api.ts';

export type Status = 'initial' | 'routing' | 'rerouting' | 'search' | 'suboptimal' | 'optimal';
const empty = (day: string): Scenario => ({ name: 'Сценарий', jobs: [], engineers: [], transit_date: format(new Date(day), 'yyyy-MM-dd') });
const forDay = (scenario: ApiScenario, day: string): Scenario => {
  const date = format(new Date(day), 'yyyy-MM-dd');
  return fromScenario({ ...scenario, transit_date: date,
    routing: scenario.transit_date === date ? scenario.routing : undefined }, day);
};
export const navigationFactory = () => {
  const key = startOfDay(new Date()).toISOString();
  const $date = atom(key);
  const $statusMap = map<Record<string, Status>>({ [key]: 'initial' });
  const $scenarioMap = map<Record<string, Scenario>>({ [key]: empty(key) });
  const $resultMap = map<Record<string, PlanResult | null>>({ [key]: null });
  const rawResults = new Map<string, ApiResult>();
  const $viewMode = atom<'live' | 'plan'>('plan');
  const $busy = atom(false);
  const $error = atom('');
  const $datasets = atom<string[]>([]);
  const $norms = atom<Norm[]>([]);
  const $seconds = atom(5);
  const $solver = atom<'fast' | 'exact'>('exact');
  const $time = atom(720);
  const $playhead = atom(720);
  // Domain events stay minute-based; the map renders the continuous playback clock.
  $time.subscribe(value => $playhead.set(value));
  const $baseline = atom(false);
  const $needsRoutingMap = map<Record<string, boolean>>({});
  const ready = <T>(value: T) => ({ state: 'ready' as const, value });
  const $scenario = computed([$date, $scenarioMap], (day, values) => ready(values[day] ?? empty(day)));
  const $result = computed([$date, $resultMap], (day, values) => ready(values[day] ?? null));
  const $status = computed([$date, $statusMap], (day, values) => ready(values[day] ?? 'initial'));
  const $historyLocked = computed([$date, $resultMap], (day, values) => !!values[day]?.lastEventTime || !!values[day]?.changes.length);
  const $needsRouting = computed([$date, $needsRoutingMap], (day, values) => !!values[day]);
  const accept = (scenario: Scenario, day = $date.get()) => {
    $scenarioMap.setKey(day, scenario);
    $resultMap.setKey(day, null);
    $statusMap.setKey(day, 'initial');
    $needsRoutingMap.setKey(day, false);
    rawResults.delete(day);
    $baseline.set(false);
    $viewMode.set('plan');
  };
  const perform = async (task: (day: string) => Promise<void>) => {
    if ($busy.get()) return false;
    const day = $date.get();
    const status = $statusMap.get()[day];
    $busy.set(true); $error.set('');
    try { await task(day); return true; }
    catch (error) {
      $error.set(error instanceof Error ? error.message : String(error));
      if (['routing', 'rerouting', 'search'].includes($statusMap.get()[day])) $statusMap.setKey(day, status);
      return false;
    }
    finally { $busy.set(false); }
  };
  const prepareDay = async (day: string) => {
    const scenario = $scenarioMap.get()[day];
    if (!scenario.jobs.length || !scenario.engineers.length) return;
    $needsRoutingMap.setKey(day, true);
    $statusMap.setKey(day, 'routing');
    try {
      const prepared = await request<ApiScenario>('/api/routing', { scenario: toScenario(scenario, day), confirm_coordinates: true });
      accept(fromScenario(prepared, day), day);
    } finally { $statusMap.setKey(day, 'initial'); }
  };
  const load = (path: string) => perform(async day => {
    accept(forDay(await request<ApiScenario>(path), day), day);
    await prepareDay(day);
  });
  const importFile = (name: string, text: string) => perform(async day => {
    const parsed = name.toLowerCase().endsWith('.json') ? JSON.parse(text) : null;
    // Canonical multi-day format: { days: [{ date: "YYYY-MM-DD", scenario: {...} }] }.
    const entries: { date?: string; scenario: unknown }[] = parsed?.days
      ? (Array.isArray(parsed.days) ? parsed.days : Object.entries(parsed.days).map(([date, scenario]) => ({ date, scenario })))
      : [{ date: parsed?.date || parsed?.scenario?.transit_date || parsed?.transit_date, scenario: parsed?.scenario || parsed }];
    if (!entries.length) throw Error('Файл не содержит дней.');
    const imported: { day: string; scenario: Scenario }[] = [];
    for (const entry of entries) {
      const date = entry.date ? new Date(`${entry.date}T00:00:00`) : new Date(day);
      if (!Number.isFinite(date.getTime()) || (entry.date && format(date, 'yyyy-MM-dd') !== entry.date)) throw Error('Неверная дата в файле: ' + entry.date);
      const bucket = startOfDay(date).toISOString();
      if (imported.some(item => item.day === bucket)) throw Error('Дата повторяется в файле: ' + entry.date);
      const scenario = await request<ApiScenario>('/api/import', { name: parsed ? 'scenario.json' : name, text: parsed ? JSON.stringify(entry.scenario) : text });
      imported.push({ day: bucket, scenario: forDay(scenario, bucket) });
    }
    // Commit only after every day has passed backend validation.
    for (const item of imported) accept(item.scenario, item.day);
    $date.set(imported[0].day);
    const errors: string[] = [];
    for (const item of imported) {
      try { await prepareDay(item.day); }
      catch (error) { errors.push(`${format(new Date(item.day), 'dd.MM.yyyy')}: ${error instanceof Error ? error.message : String(error)}`); }
    }
    if (errors.length) throw Error(`Данные загружены, но не удалось подготовить дороги. ${errors.join('; ')}`);
  });
  const setScenario = (scenario: Scenario) => {
    if ($busy.get() || $historyLocked.get()) return false;
    const current = $scenario.get().value;
    // Names/addresses do not change graph costs. Keep prepared arcs across edits;
    // the backend fills newly required coverage without refreshing frozen arcs.
    const graphInput = (value: Scenario) => JSON.stringify({
      jobs: value.jobs.map(job => [job.point, job.duration, job.windowStart, job.windowEnd, job.skill, job.transport]),
      engineers: value.engineers.map(engineer => [engineer.start, engineer.shiftStart, engineer.shiftEnd, engineer.skills, engineer.transport]),
      transit_date: value.transit_date,
    });
    const changed = graphInput(current) !== graphInput(scenario);
    const needsRouting = $needsRouting.get() || (!!current.routing && changed);
    const routing = current.transit_date === scenario.transit_date ? current.routing : undefined;
    accept({ ...scenario, routing });
    $needsRoutingMap.setKey($date.get(), needsRouting);
    return true;
  };
  const solve = (event?: { kind: 'cancel'; time: number; job_id: string } | { kind: 'add_urgent'; time: number; job: ReturnType<typeof toJob> }) => perform(async day => {
    if ($needsRouting.get()) throw Error('Данные изменены: подготовьте дороги заново или явно выберите офлайн-схему.');
    if (!event && $historyLocked.get()) throw Error('История зафиксирована. Откройте исходный набор для нового плана.');
    const previous = rawResults.get(day);
    if (event && !previous) throw Error('Сначала постройте план.');
    $statusMap.setKey(day, event ? 'rerouting' : 'search');
    const result = await request<ApiResult>('/api/plan', {
      scenario: toScenario($scenarioMap.get()[day], day), seconds: $seconds.get(), mode: $solver.get(),
      ...(event && previous ? { event, last_event_time: previous.last_event_time,
        previous: { ...previous.plan, routes: previous.plan.routes.map(route => ({ ...route,
          stops: route.stops.map(({ shape: _shape, ...stop }) => stop) })) } } : {}),
    });
    rawResults.set(day, result);
    $scenarioMap.setKey(day, fromScenario(result.scenario, day));
    $resultMap.setKey(day, fromResult(result, day));
    $statusMap.setKey(day, result.stats.optimal ? 'optimal' : 'suboptimal');
  });
  let initialized = false;
  return {
    $date, $scenario, $result, $status, $viewMode, $busy, $error, $datasets, $norms,
    $seconds, $solver, $time, $playhead, $baseline, $historyLocked, $needsRouting,
    seekTime: (minute: number) => { $time.set(minute); $playhead.set(minute); },
    $dates: computed($scenarioMap, values => Object.keys(values)),
    setScenario, load, importFile,
    scenarioFromJSON: (text: string) => importFile('scenario.json', text),
    scenarioFromCSV: (text: string) => importFile('scenario.csv', text),
    startSearch: () => solve(),
    addUrgent: (job: Job) => solve({ kind: 'add_urgent', time: $time.get(), job: toJob(job, $date.get()) }),
    cancelJob: (job_id: string) => solve({ kind: 'cancel', time: $time.get(), job_id }),
    initialize: async () => {
      if (initialized) return;
      initialized = true;
      await perform(async () => {
        const [datasets, norms] = await Promise.all([request<string[]>('/api/datasets'), request<Norm[]>('/api/norms')]);
        $datasets.set(datasets); $norms.set(norms);
      });
    },
    prepare: (confirmed: boolean) => perform(async day => {
      if (!confirmed) throw Error('Подтвердите координаты заявок и баз инженеров.');
      if ($historyLocked.get()) throw Error('История зафиксирована. Откройте исходный набор.');
      await prepareDay(day);
    }),
    geocode: () => perform(async day => {
      if ($historyLocked.get()) throw Error('История зафиксирована. Откройте исходный набор.');
      const needsRouting = $needsRouting.get() || !!$scenario.get().value.routing;
      accept(fromScenario(await request<ApiScenario>('/api/geocode', { scenario: toScenario($scenario.get().value, day) }), day), day);
      $needsRoutingMap.setKey(day, needsRouting);
    }),
    useOffline: () => {
      if ($busy.get() || $historyLocked.get()) return;
      accept({ ...$scenario.get().value, routing: undefined });
      $error.set('');
    },
    exportDays: () => ({ version: 1, days: Object.entries($scenarioMap.get()).map(([day, scenario]) => ({
      date: format(new Date(day), 'yyyy-MM-dd'), scenario: toScenario(scenario, day),
    })) }),
    exportResult: () => ({ ...rawResults.get($date.get()), date: format(new Date($date.get()), 'yyyy-MM-dd') }),
    exportScenario: () => toScenario($scenario.get().value, $date.get()),
    setDate: (date: Date) => {
      if ($busy.get()) return;
      const day = startOfDay(date).toISOString();
      if (!$scenarioMap.get()[day]) accept(empty(day), day);
      $date.set(day);
      $viewMode.set('plan');
      $baseline.set(false);
      $error.set('');
      $time.set(Math.max(720, rawResults.get(day)?.last_event_time || 0));
      $playhead.set($time.get());
    },
  };
};
