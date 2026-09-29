import { render } from 'preact';
import { LocateFixed, Car, Bike, Footprints, Bus } from 'lucide-preact';
import { MapDetails } from './details';
import { useStore } from '@nanostores/preact';
import { useEffect, useMemo, useRef, useState } from 'preact/hooks';
import * as maplibregl from 'maplibre-gl';
import 'maplibre-gl/dist/maplibre-gl.css';
import workerUrl from 'maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url';
// MapLibre 6's worker must be emitted by Vite alongside the application.
maplibregl.setWorkerUrl(workerUrl);
import type { FeatureCollection, LineString } from 'geojson';
import { navigation, type Plan } from '../../entity/navigation';
import { atMinute } from '../../entity/navigation/api';
import { palette, routeLegs, engineerPosition, routeProgress, pathLength } from '../../entity/navigation/route-view';
import { editorModel } from '../../features/editor/model';

export const MapDisplay = () => {
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value;
  const baseline = useStore(navigation.$baseline);
  const live = useStore(navigation.$viewMode) === 'live';
  const playhead = useStore(navigation.$playhead);
  const [reducedMotion, setReducedMotion] = useState(() => window.matchMedia('(prefers-reduced-motion: reduce)').matches);
  const time = reducedMotion ? Math.floor(playhead) : playhead;
  useEffect(() => {
    const preference = window.matchMedia('(prefers-reduced-motion: reduce)');
    const change = () => setReducedMotion(preference.matches);
    preference.addEventListener('change', change);
    return () => preference.removeEventListener('change', change);
  }, []);
  const day = useStore(navigation.$date);
  const selectedId = useStore(editorModel.$selectedId);
  const selectedEntity = useStore(editorModel.$entity);
  const scenarioRef = useRef(scenario);
  scenarioRef.current = scenario;
  const engineerMarkers = useRef(new Map<string, maplibregl.Marker>());
  const container = useRef<HTMLDivElement>(null);
  const map = useRef<maplibregl.Map | null>(null);
  const [ready, setReady] = useState(false);
  const [fallback, setFallback] = useState(false);
  const plan = baseline ? result?.baseline : result?.plan;
  const routes = useMemo(() => plan?.routes.map(route => {
    const legs = routeLegs(scenario, route);
    return { route, legs, lengths: legs.map(pathLength), color: palette[Math.max(0, scenario.engineers.findIndex(engineer => engineer.id === route.engineerId)) % palette.length] };
  }) || [], [scenario, plan]);
  const lines = useMemo(() => routes.flatMap(({ route, legs, color }) => legs.filter(points => points.length > 1).map(points => ({ points, engineerId: route.engineerId, color }))), [routes]);
  const fit = () => {
    const instance = map.current;
    const current = scenarioRef.current;
    const points = [...current.jobs.map(job => job.point), ...current.engineers.map(engineer => engineer.start)];
    if (!instance || !points.length) return;
    const bounds = new maplibregl.LngLatBounds(); points.forEach(point => bounds.extend([point.lon, point.lat]));
    instance.fitBounds(bounds, { padding: 70, maxZoom: 14, duration: 400 });
  };
  useEffect(() => {
    if (!container.current) return;
    let instance: maplibregl.Map;
    try {
      instance = new maplibregl.Map({ container: container.current, center: [37.61, 55.75], zoom: 10,
        style: 'https://basemaps.cartocdn.com/gl/positron-gl-style/style.json',
        attributionControl: { compact: true },
      });
    } catch { setFallback(true); return; }
    map.current = instance;
    instance.addControl(new maplibregl.NavigationControl({ showCompass: false }), 'bottom-right');
    let recovering = false, configured = false;
    instance.on('error', () => {
      if (configured || recovering) return;
      recovering = true;
      instance.setStyle({ version: 8, sources: { osm: { type: 'raster', tiles: ['https://tile.openstreetmap.org/{z}/{x}/{y}.png'], tileSize: 256, attribution: '© OpenStreetMap contributors' } }, layers: [{ id: 'osm', type: 'raster', source: 'osm', paint: { 'raster-saturation': -.65 } }] });
    });
    instance.once('style.load', () => {
      configured = true;
      instance.addSource('routes', { type: 'geojson', data: { type: 'FeatureCollection', features: [] } });
      instance.addLayer({ id: 'route-halo', type: 'line', source: 'routes', layout: { 'line-cap': 'round', 'line-join': 'round' }, paint: { 'line-color': '#ffffff', 'line-width': ['+', ['get', 'width'], 3], 'line-opacity': ['get', 'opacity'] } });
      instance.addLayer({ id: 'routes', type: 'line', source: 'routes', filter: ['!=', ['get', 'remaining'], true], layout: { 'line-cap': 'round', 'line-join': 'round' }, paint: { 'line-color': ['get', 'color'], 'line-width': ['get', 'width'], 'line-opacity': ['get', 'opacity'] } });
      instance.addLayer({ id: 'route-remaining', type: 'line', source: 'routes', filter: ['==', ['get', 'remaining'], true], layout: { 'line-cap': 'round', 'line-join': 'round' }, paint: { 'line-color': '#94a3b8', 'line-width': ['get', 'width'], 'line-opacity': ['get', 'opacity'], 'line-dasharray': [1.5, 2] } });
      for (const layer of ['routes', 'route-remaining']) {
        instance.on('mouseenter', layer, () => { instance.getCanvas().style.cursor = 'pointer'; });
        instance.on('mouseleave', layer, () => { instance.getCanvas().style.cursor = ''; });
        instance.on('click', layer, event => { const id = event.features?.[0]?.properties?.engineerId; if (id) editorModel.startEditEngineer(id, 'map'); });
      }
      setReady(true);
    });
    let lastMeasured = 0;
    instance.on('render', () => {
      if (performance.now() - lastMeasured < 500) return;
      lastMeasured = performance.now();
      if (instance.getLayer('routes') && container.current?.parentElement) {
        container.current.parentElement.dataset.renderedRoutes = String(instance.queryRenderedFeatures({ layers: ['routes', 'route-remaining'] }).length);
      }
    });
    const observer = new ResizeObserver(() => instance.resize()); observer.observe(container.current);
    return () => { observer.disconnect(); instance.remove(); map.current = null; };
  }, []);
  useEffect(() => {
    if (!ready || !map.current) return;
    const points = [...scenario.jobs.map(job => job.point), ...scenario.engineers.map(engineer => engineer.start)];
    if (!points.length) return;
    const bounds = new maplibregl.LngLatBounds(); points.forEach(point => bounds.extend([point.lon, point.lat]));
    map.current.fitBounds(bounds, { padding: 70, maxZoom: 14, duration: 0 });
  }, [ready, scenario]);
  useEffect(() => {
    if (!ready) return;
    const frame = requestAnimationFrame(() => { map.current?.resize(); fit(); });
    return () => cancelAnimationFrame(frame);
  }, [ready, live]);
  useEffect(() => {
    if (!ready || !map.current) return;
    const markers: maplibregl.Marker[] = [];
    const marker = (point: [number, number], text: string, color: string, title: string, click: () => void, engineer = false) => {
      const button = document.createElement('button'); button.className = `dispatch-marker ${engineer ? 'engineer-marker' : ''}`;
      if (!engineer) button.textContent = text; button.style.setProperty('--engineer-color', color); if (!engineer) button.style.borderColor = color;
      button.title = title; button.setAttribute('aria-label', title); button.onclick = click;
      const item = new maplibregl.Marker({ element: button }).setLngLat(point).addTo(map.current!);
      markers.push(item); return item;
    };
    scenario.jobs.forEach(job => {
      const route = plan?.routes.find(route => route.stops.some(stop => stop.jobId === job.id));
      const engineerIndex = scenario.engineers.findIndex(engineer => engineer.id === route?.engineerId);
      const stopIndex = route?.stops.findIndex(stop => stop.jobId === job.id) ?? -1;
      marker([job.point.lon, job.point.lat], stopIndex >= 0 ? String(stopIndex + 1) : '•', engineerIndex >= 0 ? palette[engineerIndex % palette.length] : '#94a3b8', `${job.id}: ${job.address}`, () => editorModel.startEditJob(job.id, 'map'));
    });
    scenario.engineers.forEach((engineer, index) => {
      const item = marker([engineer.start.lon, engineer.start.lat], engineer.name.split(' ').filter(word => /[\p{L}\p{N}]/u.test(word[0] || '')).map(word => word[0]).slice(0, 2).join('') || engineer.id, palette[index % palette.length], engineer.name, () => editorModel.startEditEngineer(engineer.id, 'map'), true);
      const Icon = { car: Car, walk: Footprints, bicycle: Bike, public: Bus }[engineer.transport];
      render(<><span className="engineer-progress" hidden/><Icon size={20}/></>, item.getElement());
      item.getElement().dataset.engineerId = engineer.id;
      engineerMarkers.current.set(engineer.id, item);
    });
    return () => { markers.forEach(marker => { if (marker.getElement().classList.contains("engineer-marker")) render(null, marker.getElement()); marker.remove(); }); engineerMarkers.current.clear(); };
  }, [ready, scenario, plan]);
  useEffect(() => {
    if (ready) map.current?.setPaintProperty('routes', 'line-dasharray', scenario.routing ? [1, 0] : [2, 2]);
  }, [ready, scenario.routing]);
  useEffect(() => {
    if (!ready || !map.current) return;
    const instance = map.current;
    const target = +atMinute(0, day) + time * 60_000;
    const paint = (timestamp: number) => {
      const moment = new Date(timestamp);
      const data: FeatureCollection<LineString> = { type: 'FeatureCollection', features: [] };
      const progress = new Map<string, number | null>();
      for (const { route, legs, lengths, color } of routes) {
        const view = routeProgress(route, legs, moment, lengths);
        progress.set(route.engineerId, view.percent);
        const properties = { engineerId: route.engineerId, color, width: selectedEntity === 'engineer' && selectedId === route.engineerId ? 6 : 4, opacity: selectedEntity === 'engineer' && selectedId && selectedId !== route.engineerId ? .18 : .95 };
        (live ? view.paths : legs.map(points => ({ travelled: points, remaining: [] }))).forEach(path => {
          for (const part of ['travelled', 'remaining'] as const) {
            if (path[part].length > 1) data.features.push({ type: 'Feature', properties: { ...properties, remaining: part === 'remaining' }, geometry: { type: 'LineString', coordinates: path[part] } });
          }
        });
      }
      (instance.getSource('routes') as maplibregl.GeoJSONSource).setData(data);
      scenario.engineers.forEach(engineer => {
        const marker = engineerMarkers.current.get(engineer.id);
        if (!marker) return;
        const prepared = routes.find(item => item.route.engineerId === engineer.id);
        const position = live ? engineerPosition(scenario, engineer, prepared?.route, moment, prepared?.legs) : { point: [engineer.start.lon, engineer.start.lat] as [number, number], status: 'База' };
        marker.setLngLat(position.point);
        const element = marker.getElement();
        const percent = progress.get(engineer.id);
        const label = element.querySelector<HTMLElement>('.engineer-progress')!;
        label.hidden = !live || percent == null;
        label.textContent = percent == null ? '' : `${percent}%`;
        label.title = 'Пройденная доля пути по маршруту';
        element.title = `${engineer.name} · ${position.status}${live && percent != null ? ` · маршрут ${percent}%` : ''}`;
        element.setAttribute('aria-label', element.title);
        element.dataset.status = position.status;
      });
    };
    paint(target);
  }, [ready, scenario, routes, live, time, day, selectedId, selectedEntity]);
  return <div className="relative flex-1 min-h-[260px] bg-slate-100" data-testid="map" data-route-count={lines.length}>
    <div ref={container} style={{ position: 'absolute', inset: 0 }}/>
    <MapDetails/>
    {fallback && <OfflineMap lines={lines} scenario={scenario} plan={plan} live={live} time={new Date(+atMinute(0, day) + time * 60_000)}/>}
    {!scenario.jobs.length && !scenario.engineers.length && <div className="absolute inset-0 grid place-items-center pointer-events-none"><div className="rounded-xl bg-white/95 p-6 shadow-sm text-center"><b>На этот день данных пока нет</b><p className="text-sm text-gray-500 mt-2">Загрузите набор или добавьте заявки и инженеров слева.</p></div></div>}
    <button className="map-fit" aria-label="Показать все маршруты" onClick={fit}><LocateFixed size={16}/></button>
    <div className="map-legend"><i/>{baseline ? 'Базовый план · ' : ''}{scenario.routing ? 'Реальные дороги' : 'Офлайн-схема · прямые линии'}{live ? ' · моделирование по плану' : ''}</div>
  </div>;
};
function OfflineMap({ lines, scenario, plan, live, time }: { plan?: Plan; live: boolean; time: Date; lines: { points: [number, number][]; color: string; engineerId: string }[]; scenario: ReturnType<typeof navigation.$scenario.get>['value'] }) {
  const points = [...scenario.jobs.map(job => [job.point.lon, job.point.lat]), ...scenario.engineers.map(engineer => [engineer.start.lon, engineer.start.lat]), ...lines.flatMap(line => line.points)];
  const xs = points.map(point => point[0]), ys = points.map(point => point[1]);
  const minX = Math.min(...xs), maxX = Math.max(...xs), minY = Math.min(...ys), maxY = Math.max(...ys);
  const project = (point: number[]) => [40 + 920 * (point[0] - minX) / (maxX - minX || 1), 560 - 520 * (point[1] - minY) / (maxY - minY || 1)];
  const paths = live ? (plan?.routes.flatMap(route => {
    const color = palette[Math.max(0, scenario.engineers.findIndex(e => e.id === route.engineerId)) % palette.length];
    return routeProgress(route, routeLegs(scenario, route), time).paths.flatMap(path => [
      { points: path.travelled, color, engineerId: route.engineerId, remaining: false },
      { points: path.remaining, color: '#94a3b8', engineerId: route.engineerId, remaining: true },
    ]).filter(path => path.points.length > 1);
  }) || []) : lines.map(line => ({ ...line, remaining: false }));
  return <svg className="absolute inset-0 w-full h-full" viewBox="0 0 1000 600" role="img" aria-label="Схема маршрутов без подложки">
    {paths.map((line, i) => <polyline key={i} points={line.points.map(point => project(point).join(',')).join(' ')} fill="none" stroke={line.color} strokeWidth="3" strokeDasharray={line.remaining ? "5 6" : undefined} onClick={() => editorModel.startEditEngineer(line.engineerId, 'map')}/>)}
    {scenario.jobs.map(job => { const [x, y] = project([job.point.lon, job.point.lat]); return <g key={job.id} onClick={() => editorModel.startEditJob(job.id, 'map')}><circle cx={x} cy={y} r="6" fill="#e5b700"/><text x={x + 10} y={y} fontSize="12">{job.id}</text></g>; })}
    {scenario.engineers.map((engineer, index) => {
      const route = plan?.routes.find(route => route.engineerId === engineer.id);
      const position = live ? engineerPosition(scenario, engineer, route, time).point : [engineer.start.lon, engineer.start.lat];
      const [x, y] = project(position);
      const percent = live && route ? routeProgress(route, routeLegs(scenario, route), time).percent : null;
      const Icon = { car: Car, walk: Footprints, bicycle: Bike, public: Bus }[engineer.transport];
      return <g key={engineer.id} role="button" tabIndex={0} aria-label={engineer.name} onClick={() => editorModel.startEditEngineer(engineer.id, 'map')} onKeyDown={e => { if (e.key === 'Enter') editorModel.startEditEngineer(engineer.id, 'map'); }}>
        <circle cx={x} cy={y} r="18" fill={palette[index % palette.length]} stroke="white" strokeWidth="3"/>
        <Icon x={x - 10} y={y - 10} width={20} height={20} color="white"/>
        {percent != null && <><rect x={x - 22} y={y - 49} width="44" height="24" rx="6" fill="white" stroke="#263244"/><text x={x} y={y - 33} textAnchor="middle" fontSize="12" fill="#263244">{percent}%</text></>}
      </g>;
    })}
  </svg>;
}
