import { useStore } from '@nanostores/preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import { X, Pencil, UserRound, MapPin } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { atMinute } from '../../entity/navigation/api';
import { engineerPosition } from '../../entity/navigation/route-view';
import { editorModel } from '../../features/editor/model';
import { EngineerEditor } from '../../features/editor/engineer';
import { JobEditor } from '../../features/editor/job';
import { Button } from '../../shared/ui/button';
import { skillNames, transportNames } from '../settings/engineers';
import { formatTime } from '../settings/jobs/utils';
import { AssignmentReasons } from './assignment-reasons';

export const MapDetails = () => {
  const placement = useStore(editorModel.$placement);
  const mode = useStore(editorModel.$mode);
  const entity = useStore(editorModel.$entity);
  const id = useStore(editorModel.$selectedId);
  const day = useStore(navigation.$date);
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value;
  const baseline = useStore(navigation.$baseline);
  const live = useStore(navigation.$viewMode) === 'live';
  const time = useStore(navigation.$time);
  const locked = useStore(navigation.$historyLocked);
  const busy = useStore(navigation.$busy);
  const plan = baseline ? result?.baseline : result?.plan;
  const [editing, setEditing] = useState(false);
  const close = useRef<HTMLButtonElement>(null);
  const open = placement === 'map' && mode === 'edit';
  useEffect(() => { setEditing(false); if (open) close.current?.focus({ preventScroll: true }); }, [id, entity, open]);
  useEffect(() => {
    if (!open) return;
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') editorModel.close(); };
    window.addEventListener('keydown', escape);
    return () => window.removeEventListener('keydown', escape);
  }, [open]);
  if (!open) return null;
  const job = entity === 'job' ? scenario.jobs.find(job => job.id === id) : undefined;
  const engineer = entity === 'engineer' ? scenario.engineers.find(engineer => engineer.id === id) : undefined;
  if (!job && !engineer) return null;
  const route = plan?.routes.find(route => engineer ? route.engineerId === engineer.id : route.stops.some(stop => stop.jobId === id));
  const stop = job ? route?.stops.find(stop => stop.jobId === id) : undefined;
  const assigned = scenario.engineers.find(engineer => engineer.id === route?.engineerId);
  const reason = plan?.unassigned.find(item => item.jobId === id)?.reason;
  const current = engineer && engineerPosition(scenario, engineer, route, atMinute(time, day));
  return <section role="dialog" aria-modal="false" aria-label={job ? `Заявка ${job.id}` : `Инженер ${engineer!.name}`} data-testid="map-details" className="map-details">
    {editing ? <div className="flex-1 min-h-0">{job ? <JobEditor/> : <EngineerEditor/>}</div> : <>
      <div className="flex items-start gap-3 border-b pb-3">
        <span className="rounded-full bg-yellow-300 p-2">{job ? <MapPin size={20}/> : <UserRound size={20}/>}</span>
        <div className="flex-1 min-w-0"><h2 className="font-medium">{job ? `Заявка ${job.id}` : engineer!.name}</h2><p className="text-xs text-gray-500">{job ? (job.urgent ? 'Срочная заявка' : 'Заявка') : engineer!.id}{baseline ? ' · базовый план' : ''}</p></div>
        <button ref={close} onClick={editorModel.close} aria-label="Закрыть детали" className="rounded-lg p-1 hover:bg-gray-100"><X size={18}/></button>
      </div>
      <div className="overflow-y-auto min-h-0 flex-1 text-sm flex flex-col gap-4">
        {job && <>
          <p className="font-medium">{job.address}</p>
          <dl className="metrics-grid"><dt>Окно</dt><dd>{formatTime(job.windowStart, job.windowEnd)}</dd><dt>Работа</dt><dd>{job.duration} мин</dd><dt>Навык</dt><dd>{skillNames[job.skill]}</dd><dt>Транспорт</dt><dd>{job.transport ? transportNames[job.transport] : 'Любой'}</dd><dt>Статус</dt><dd>{!plan ? 'Ожидает расчёта' : stop ? 'Назначена' : 'Не назначена'}</dd></dl>
          {assigned && <button className="text-left rounded-lg bg-amber-50 p-3" onClick={() => editorModel.startEditEngineer(assigned.id, 'map')}><span className="text-xs text-gray-500 block">Исполнитель</span><b>{assigned.name}</b>{stop && <span className="block mt-1">Обслуживание {formatTime(stop.start, stop.end)}</span>}</button>}
          {stop && assigned && <AssignmentReasons job={job} engineer={assigned} stop={stop} baseline={baseline}/>}
          {reason && <p className="rounded-lg bg-orange-50 p-3">{reason}</p>}
          {job.geocodeMatch && <p className="text-xs text-gray-500">{job.geocodeMatch}</p>}
          {live && result && !baseline && <Button variant="outline" disabled={busy || !!stop && stop.departure < atMinute(time, day)} onClick={async () => { if (await navigation.cancelJob(id)) editorModel.close(); }}>Отменить заявку в выбранное время</Button>}
        </>}
        {engineer && <>
          <dl className="metrics-grid"><dt>Смена</dt><dd>{formatTime(engineer.shiftStart, engineer.shiftEnd)}</dd><dt>Транспорт</dt><dd>{transportNames[engineer.transport]}</dd><dt>Заявок</dt><dd>{route?.stops.length || 0}</dd><dt>Пробег</dt><dd>{((route?.distance || 0) / 1000).toFixed(1)} км</dd>{live && <><dt>Сейчас</dt><dd>{current?.status}</dd></>}</dl>
          <p><span className="text-gray-500">Навыки: </span>{engineer.skills.map(skill => skillNames[skill]).join(', ')}</p>
          <div><h3 className="font-medium mb-2">Расписание</h3>{route?.stops.length ? route.stops.map((stop, index) => <button key={stop.jobId} className="w-full text-left border rounded-lg p-2 mb-2 hover:bg-amber-50" onClick={() => editorModel.startEditJob(stop.jobId, 'map')}><b>{index + 1}. {stop.jobId}</b><span className="float-right text-xs">{formatTime(stop.start, stop.end)}</span><span className="block text-xs text-gray-500 mt-1">{scenario.jobs.find(job => job.id === stop.jobId)?.address}</span></button>) : <p className="text-gray-500">{plan ? 'Назначенных заявок нет' : 'Постройте план, чтобы увидеть расписание'}</p>}</div>
        </>}
      </div>
      {!locked && <Button variant="outline" disabled={busy} onClick={() => setEditing(true)}><Pencil size={16}/>Редактировать</Button>}
    </>}
  </section>;
};
