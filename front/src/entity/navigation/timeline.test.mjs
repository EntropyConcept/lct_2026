import assert from 'node:assert/strict';
import { test } from 'node:test';
import { atMinute } from './api.ts';
import { scheduleSegments } from './timeline.ts';
const day = new Date(2026, 8, 29).toISOString();
const stop = (id, departure, arrival, start, end) => ({ jobId:id, departure:atMinute(departure,day), arrival:atMinute(arrival,day), start:atMinute(start,day), end:atMinute(end,day) });
test('dense schedule retains short trips and waits without overlaps or zero-width events', () => {
  const route = { stops:[stop('A',540,541,550,620),stop('B',620,622,622,692)] };
  const segments = scheduleSegments(route,540,1440,day);
  assert.deepEqual(segments.map(s=>[s.kind,s.start,s.end]), [['travel',540,541],['wait',541,550],['work',550,620],['travel',620,622],['work',622,692],['free',692,1440]]);
  assert.equal(segments[1].jobId, 'A');
  assert.equal(segments.reduce((sum,s)=>sum+s.end-s.start,0),900);
});
test('unused engineer has one free interval; initial wait and colocated work remain distinct', () => {
  assert.deepEqual(scheduleSegments(undefined,540,1080,day).map(s=>[s.kind,s.start,s.end]), [['free',540,1080]]);
  assert.deepEqual(scheduleSegments({stops:[stop('A',570,570,570,600)]},540,600,day).map(s=>[s.kind,s.start,s.end]), [['wait',540,570],['work',570,600]]);
});
