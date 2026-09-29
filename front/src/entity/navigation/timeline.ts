import type { Route } from './types';
import { minutes } from './api.ts';
export type ScheduleSegment = { start: number; end: number; title: string; kind: 'travel' | 'work' | 'wait' | 'free'; jobId?: string };
export function scheduleSegments(route: Route | undefined, shiftStart: number, shiftEnd: number, day: string): ScheduleSegment[] {
  const values: ScheduleSegment[] = [];
  const add = (start: number, end: number, title: string, kind: ScheduleSegment['kind'], jobId?: string) => {
    if (end > start) values.push({ start, end, title, kind, jobId });
  };
  let previous = shiftStart;
  for (const stop of route?.stops || []) {
    const departure = minutes(stop.departure, day), arrival = minutes(stop.arrival, day), start = minutes(stop.start, day), end = minutes(stop.end, day);
    add(previous, departure, 'Ожидание', 'wait');
    add(departure, arrival, 'Переезд', 'travel', stop.jobId);
    add(arrival, start, 'Ожидание', 'wait', stop.jobId);
    add(start, end, stop.jobId, 'work', stop.jobId);
    previous = end;
  }
  add(previous, shiftEnd, 'Свободен', 'free');
  return values;
}
