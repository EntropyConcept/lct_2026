import { useStore } from '@nanostores/preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import { Play, Pause, SkipBack, Minus, Plus, LocateFixed, ArrowRight, Clock3 } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { atMinute, minutes } from '../../entity/navigation/api';
import { clock, engineerPosition, palette } from '../../entity/navigation/route-view';
import { scheduleSegments } from '../../entity/navigation/timeline';
import { $playing } from '../../features/mode-switch/playback';
import { editorModel } from '../../features/editor/model';

export const Timeline = () => {
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value!;
  const time = useStore(navigation.$time);
  const day = useStore(navigation.$date);
  const busy = useStore(navigation.$busy);
  const selected = useStore(editorModel.$selectedId);
  const editorMode = useStore(editorModel.$mode);
  const playing = useStore($playing);
  const setPlaying = (value: boolean) => $playing.set(value);
  useEffect(() => () => $playing.set(false), []);
  const [speed, setSpeed] = useState(5);
  const [zoom, setZoom] = useState(1);
  const [availableWidth, setAvailableWidth] = useState(950);
  const [hint, setHint] = useState('Выберите событие, чтобы открыть детали на карте');
  const scroll = useRef<HTMLDivElement>(null);
  const start = Math.floor(Math.min(...scenario.engineers.map(e => minutes(e.shiftStart))) / 60) * 60;
  const end = Math.max(start + 60, ...scenario.engineers.map(e => minutes(e.shiftEnd, day)));
  const span = end - start;
  const scale = Math.max(.75, (availableWidth - 180) / span) * zoom;
  const width = span * scale;
  const ticks = Array.from({ length: Math.floor(span / 60) + 1 }, (_, i) => start + i * 60);
  const seek = (value: number) => { setPlaying(false); navigation.seekTime(Math.max(start, Math.min(end, value))); };
  const center = () => scroll.current?.scrollTo({ left: Math.max(0, (navigation.$time.get() - start) * scale - (scroll.current.clientWidth - 180) / 2), behavior: 'smooth' });
  useEffect(() => {
    const element = scroll.current!;
    const observer = new ResizeObserver(() => setAvailableWidth(element.clientWidth));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { if (busy || editorMode !== 'none') setPlaying(false); }, [busy, editorMode, selected]);
  useEffect(() => setPlaying(false), [result, day]);
  useEffect(() => { navigation.$time.set(Math.max(start, Math.min(end, navigation.$time.get()))); }, [start, end]);
  useEffect(() => {
    if (!playing || busy) return;
    let frame = 0, previous = performance.now();
    let current = navigation.$playhead.get();
    const tick = (now: number) => {
      if (navigation.$busy.get() || editorModel.$mode.get() !== 'none') { setPlaying(false); return; }
      if (document.hidden) previous = now;
      if (now - previous >= 30) {
        current = Math.min(end, current + (now - previous) * speed / 1000);
        previous = now;
        navigation.$time.set(Math.floor(current));
        navigation.$playhead.set(current);
        if (current >= end) { setPlaying(false); return; }
      }
      frame = requestAnimationFrame(tick);
    };
    const visibility = () => { previous = performance.now(); };
    document.addEventListener('visibilitychange', visibility);
    frame = requestAnimationFrame(tick);
    return () => { cancelAnimationFrame(frame); document.removeEventListener('visibilitychange', visibility); };
  }, [playing, busy, speed, end]);
  const counts = scenario.engineers.reduce((acc, e) => {
    const status = engineerPosition(scenario, e, result.plan.routes.find(r => r.engineerId === e.id), atMinute(time, day)).status;
    if (status === 'В пути') acc.travel++; else if (status === 'На месте') acc.work++;
    return acc;
  }, { travel: 0, work: 0 });
  return <section className="live-panel" data-testid="timeline" aria-label="Расписание инженеров">
    <div className="live-toolbar">
      <div className="live-heading"><span className="live-dot"/><b>Ход дня</b><span className="live-simulation">Моделирование</span></div>
      <div className="live-playback">
        <button className="live-icon" aria-label="В начало дня" disabled={busy} onClick={() => seek(start)}><SkipBack size={15}/></button>
        <button className="live-play" data-testid="live-play" aria-label={playing ? 'Приостановить' : 'Воспроизвести день'} disabled={busy} onClick={() => { if (playing) setPlaying(false); else { editorModel.close(); if (time >= end) navigation.$time.set(start); setPlaying(true); } }}>{playing ? <Pause size={16}/> : <Play size={16}/>}</button>
        <input className="live-clock" type="time" aria-label="Текущее время" value={clock(Math.min(time, 1439))} disabled={busy} onInput={event => { const [h, m] = event.currentTarget.value.split(':').map(Number); if (Number.isFinite(h + m)) seek(h * 60 + m); }}/>
        <input className="live-scrubber" type="range" aria-label="Время событий" min={start} max={end} step="1" value={time} disabled={busy} style={{ '--progress': `${(time - start) / span * 100}%` }} onInput={e => seek(Number(e.currentTarget.value))}/>
        <select className="live-speed" aria-label="Скорость воспроизведения" value={speed} onChange={e => setSpeed(Number(e.currentTarget.value))}>{[1, 5, 15].map(value => <option value={value} key={value}>{value} мин/с</option>)}</select>
      </div>
      <div className="live-zoom"><button className="live-icon" aria-label="Уменьшить масштаб расписания" disabled={zoom <= .75} onClick={() => setZoom(Math.max(.75, zoom - .25))}><Minus size={15}/></button><span>{Math.round(zoom * 100)}%</span><button className="live-icon" aria-label="Увеличить масштаб расписания" disabled={zoom >= 3} onClick={() => setZoom(Math.min(3, zoom + .25))}><Plus size={15}/></button><button className="live-icon" aria-label="Показать текущее время" onClick={center}><LocateFixed size={16}/></button></div>
    </div>
    <div className="live-summary"><span>{scenario.engineers.length} инженеров</span><span><i className="status-dot travel"/>{counts.travel} в пути</span><span><i className="status-dot work"/>{counts.work} на месте</span><span className="live-legend"><i className="legend-work"/>Работа<i className="legend-travel"/>Переезд<i className="legend-wait"/>Ожидание<i className="legend-free"/>Свободен</span></div>
    <div className="live-scroll" ref={scroll}>
      <div className="live-schedule" style={{ width: width + 180 }}>
        <div className="live-axis"><div className="live-label">Инженеры <span>{scenario.engineers.length}</span></div><div className="live-hours" style={{ width }} onClick={e => { if (!busy) seek(Math.round(start + (e.clientX - e.currentTarget.getBoundingClientRect().left) / scale)); }}>{ticks.map(tick => <span style={{ left: (tick - start) * scale }} key={tick}>{clock(tick)}</span>)}<b className="live-time-tag" style={{ left: Math.max(20, Math.min(width - 20, (time - start) * scale)) }}>{clock(time)}</b></div></div>
        {scenario.engineers.map((engineer, index) => {
          const route = result.plan.routes.find(route => route.engineerId === engineer.id);
          const position = engineerPosition(scenario, engineer, route, atMinute(time, day));
          return <div className={`live-row ${selected === engineer.id ? 'selected' : ''}`} key={engineer.id}>
            <button className="live-label live-person" onClick={() => { setPlaying(false); editorModel.startEditEngineer(engineer.id, 'map'); }} title={engineer.name}>
              <span className="live-avatar" style={{ '--engineer-color': palette[index % palette.length] }}>{engineer.name.split(' ').filter(word => /[\p{L}\p{N}]/u.test(word[0] || '')).map(word => word[0]).slice(0, 2).join('')}</span>
              <span className="live-person-copy"><b>{engineer.name}</b><small>{position.status} · {route?.stops.length || 0} заявок</small></span>
            </button>
            <div className="live-track" style={{ width, backgroundSize: `${60 * scale}px 100%` }}>
              {scheduleSegments(route, minutes(engineer.shiftStart), minutes(engineer.shiftEnd, day), day).map((segment, i) => {
                const label = `${engineer.name} · ${segment.title} · ${clock(segment.start)}–${clock(segment.end)} · ${segment.end - segment.start} мин`;
                return <button key={i} className={`live-event event-${segment.kind} ${time >= segment.start && time < segment.end ? 'current' : ''}`} style={{ left: (segment.start - start) * scale, width: Math.max(1, (segment.end - segment.start) * scale - 2) }} title={label} aria-label={label} disabled={busy} onMouseEnter={() => setHint(label)} onFocus={() => setHint(label)} onClick={() => { seek(segment.start); if (segment.jobId) editorModel.startEditJob(segment.jobId, 'map'); }}><span className="event-icon">{segment.kind === 'travel' ? <ArrowRight size={12}/> : segment.kind === 'wait' ? <Clock3 size={12}/> : null}</span><b className="event-title">{segment.title}</b><span className="event-time">{clock(segment.start)}–{clock(segment.end)}</span></button>;
              })}
              <div className="live-cursor" style={{ left: (time - start) * scale }}/>
            </div>
          </div>;
        })}
      </div>
    </div>
    <div className="live-hint" role="status">{hint}</div>
  </section>;
};
