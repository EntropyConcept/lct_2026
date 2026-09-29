import type { Engineer } from './worker';
import type { Route, Scenario } from './types';
export type Coordinate = [number, number];
export const palette = ['#e5b700', '#2563eb', '#16a34a', '#e05b35', '#9333ea', '#0891b2'];
export const clock = (minute: number) => `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}`;
export function decodePolyline6(value = ''): Coordinate[] {
  let index = 0, lat = 0, lon = 0;
  const points: Coordinate[] = [];
  const read = () => {
    let result = 0, shift = 0, byte: number;
    do {
      if (index >= value.length || shift > 30) throw Error('Invalid route geometry');
      byte = value.charCodeAt(index++) - 63;
      if (byte < 0 || byte > 63) throw Error('Invalid route geometry');
      result |= (byte & 31) << shift; shift += 5;
    } while (byte >= 32);
    return result & 1 ? ~(result >> 1) : result >> 1;
  };
  try { while (index < value.length) { lat += read(); lon += read(); if (Math.abs(lat) > 90e6 || Math.abs(lon) > 180e6) throw Error('Invalid coordinates'); points.push([lon / 1e6, lat / 1e6]); } }
  catch { return []; }
  return points;
}
export function routeLegs(scenario: Scenario, route: Route): Coordinate[][] {
  const engineer = scenario.engineers.find(engineer => engineer.id === route.engineerId);
  if (!engineer) return [];
  let previous: Coordinate = [engineer.start.lon, engineer.start.lat];
  return route.stops.map(stop => {
    const job = scenario.jobs.find(job => job.id === stop.jobId);
    if (!job) return [];
    const end: Coordinate = [job.point.lon, job.point.lat];
    const shape = scenario.routing ? decodePolyline6(stop.shape) : [previous, end];
    previous = end;
    return shape;
  });
}
export function pointAlong(points: Coordinate[], progress: number): Coordinate | null {
  if (!points.length) return null;
  const lengths = points.slice(1).map((p, i) => Math.hypot((p[0] - points[i][0]) * Math.cos(p[1] * Math.PI / 180), p[1] - points[i][1]));
  let remaining = lengths.reduce((sum, length) => sum + length, 0) * Math.max(0, Math.min(1, progress));
  for (let i = 0; i < lengths.length; i++) {
    if (remaining <= lengths[i] && lengths[i] > 0) {
      const t = remaining / lengths[i];
      return [points[i][0] + (points[i + 1][0] - points[i][0]) * t, points[i][1] + (points[i + 1][1] - points[i][1]) * t];
    }
    remaining -= lengths[i];
  }
  return points.at(-1)!;
}
export function engineerPosition(scenario: Scenario, engineer: Engineer, route: Route | undefined, time: Date, legs = route ? routeLegs(scenario, route) : []): { point: Coordinate; status: string } {
  let point: Coordinate = [engineer.start.lon, engineer.start.lat];
  if (!route?.stops.length) return { point, status: 'Свободен' };
  for (let i = 0; i < route.stops.length; i++) {
    const stop = route.stops[i];
    if (time < stop.departure) return { point, status: 'Ожидает' };
    if (time < stop.arrival) return { point: pointAlong(legs[i], (+time - +stop.departure) / (+stop.arrival - +stop.departure)) || point, status: 'В пути' };
    const job = scenario.jobs.find(job => job.id === stop.jobId);
    if (job) point = [job.point.lon, job.point.lat];
    if (time < stop.start) return { point, status: 'Ожидает' };
    if (time < stop.end) return { point, status: 'На месте' };
  }
  return { point, status: 'Маршрут завершён' };
}


const edgeLength = (a: Coordinate, b: Coordinate) => Math.hypot((b[0] - a[0]) * Math.cos(b[1] * Math.PI / 180), b[1] - a[1]);
export function pathLength(points: Coordinate[]): number {
  let length = 0;
  for (let i = 1; i < points.length; i++) length += edgeLength(points[i - 1], points[i]);
  return length;
}

/** Split at precisely the same distance interpolation used by the moving marker. */
export function splitPath(points: Coordinate[], progress: number, length = pathLength(points)): { travelled: Coordinate[]; remaining: Coordinate[] } {
  if (points.length < 2 || progress <= 0) return { travelled: [], remaining: points };
  if (progress >= 1) return { travelled: points, remaining: [] };
  let distance = length * progress;
  for (let i = 1; i < points.length; i++) {
    const length = edgeLength(points[i - 1], points[i]);
    if (length > 0 && distance <= length) {
      const t = distance / length;
      const point: Coordinate = [points[i - 1][0] + (points[i][0] - points[i - 1][0]) * t, points[i - 1][1] + (points[i][1] - points[i - 1][1]) * t];
      return { travelled: [...points.slice(0, i), point], remaining: [point, ...points.slice(i)] };
    }
    distance -= length;
  }
  return { travelled: points, remaining: [] };
}

/** Percentage of the route's mapped travel distance, excluding service/wait time. */
export function routeProgress(route: Route, legs: Coordinate[][], time: Date, lengths = legs.map(pathLength)) {
  let total = 0, travelled = 0;
  const paths = route.stops.map((stop, index) => {
    const points = legs[index] || [];
    const length = lengths[index] || 0;
    const fraction = time < stop.departure ? 0 : time >= stop.arrival ? 1 : (+time - +stop.departure) / (+stop.arrival - +stop.departure);
    total += length; travelled += length * fraction;
    return splitPath(points, fraction, length);
  });
  return { paths, percent: total > 0 ? Math.round(travelled / total * 100) : null };
}
