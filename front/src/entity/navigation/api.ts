import { startOfDay } from 'date-fns';
import type { Job } from './job';
import type { Engineer } from './worker';
import type { Scenario, Plan, PlanResult, SolverStats } from './types';

export type ApiJob = Omit<Job, 'windowStart' | 'windowEnd' | 'jobType' | 'geocodeMatch'> & {
  window_start: number; window_end: number; work_type?: string; geocode_match?: string;
};
export type ApiEngineer = Omit<Engineer, 'shiftStart' | 'shiftEnd'> & { shift_start: number; shift_end: number; already_used?: boolean };
export type ApiScenario = Omit<Scenario, 'jobs' | 'engineers'> & { jobs: ApiJob[]; engineers: ApiEngineer[] };
export interface ApiStop { job_id: string; departure: number; arrival: number; start: number; end: number; distance_m: number; shape?: string; explanation: string }
export interface ApiPlan {
  routes: { engineer_id: string; stops: ApiStop[]; distance_m: number }[];
  unassigned: { job_id: string; reason: string }[];
  metrics: { unassigned: number; urgent_unassigned: number; engineers: number; distance_m: number };
}
export interface ApiResult { scenario: ApiScenario; plan: ApiPlan; baseline: ApiPlan; stats: SolverStats; changes: string[]; last_event_time: number }
export interface Norm { key: string; name: string; service: number }

export async function request<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, body === undefined ? undefined : {
    method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body),
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || `HTTP ${response.status}`);
  return value as T;
}
export const atMinute = (minute: number, date: string | Date): Date => {
  const value = startOfDay(new Date(date));
  value.setMinutes(minute);
  return value;
};
export const minutes = (date: Date, day?: string): number => {
  if (day && startOfDay(date).getTime() > startOfDay(new Date(day)).getTime()) return 1440;
  return date.getHours() * 60 + date.getMinutes();
};
export function fromScenario(value: ApiScenario, day: string): Scenario {
  return { ...value,
    jobs: value.jobs.map(({ window_start, window_end, work_type, geocode_match, ...job }) => ({
      ...job, windowStart: atMinute(window_start, day), windowEnd: atMinute(window_end, day),
      jobType: (work_type || job.skill) as Job['jobType'], geocodeMatch: geocode_match,
    })),
    engineers: value.engineers.map(({ shift_start, shift_end, ...engineer }) => ({
      ...engineer, shiftStart: atMinute(shift_start, day), shiftEnd: atMinute(shift_end, day),
    })),
  };
}
export function toJob({ windowStart, windowEnd, jobType, geocodeMatch, ...job }: Job, day: string): ApiJob {
  return { ...job, window_start: minutes(windowStart, day), window_end: minutes(windowEnd, day), work_type: jobType, geocode_match: geocodeMatch || undefined };
}
export function toScenario(value: Scenario, day: string): ApiScenario {
  return { ...value, jobs: value.jobs.map(job => toJob(job, day)),
    engineers: value.engineers.map(({ shiftStart, shiftEnd, ...engineer }) => ({
      ...engineer, shift_start: minutes(shiftStart, day), shift_end: minutes(shiftEnd, day),
    })),
  };
}
export function fromPlan(plan: ApiPlan, day: string): Plan {
  return {
    routes: plan.routes.map(route => ({ engineerId: route.engineer_id, distance: route.distance_m,
      stops: route.stops.map(stop => ({ jobId: stop.job_id, departure: atMinute(stop.departure, day),
        arrival: atMinute(stop.arrival, day), start: atMinute(stop.start, day), end: atMinute(stop.end, day),
        shape: stop.shape, explanation: stop.explanation })),
    })),
    unassigned: plan.unassigned.map(job => ({ jobId: job.job_id, reason: job.reason })),
    metrics: { unassigned: plan.metrics.unassigned, urgentUnassigned: plan.metrics.urgent_unassigned,
      engineers: plan.metrics.engineers, distance: plan.metrics.distance_m },
  };
}
export function fromResult(value: ApiResult, day: string): PlanResult {
  return { ...value, scenario: fromScenario(value.scenario, day), plan: fromPlan(value.plan, day),
    baseline: fromPlan(value.baseline, day), lastEventTime: value.last_event_time ? atMinute(value.last_event_time, day) : null };
}
