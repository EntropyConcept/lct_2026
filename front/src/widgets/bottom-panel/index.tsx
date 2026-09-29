import { useStore } from '@nanostores/preact';
import { useState } from 'preact/hooks';
import { Car, Clock, GraduationCap, MapPin, Flag } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { minutes } from '../../entity/navigation/api';
import { clock } from '../../entity/navigation/route-view';
import { editorModel } from '../../features/editor/model';
import { transportNames, skillNames } from '../settings/engineers';
import { formatTime } from '../settings/jobs/utils';
import { Timeline } from './timeline';

export const Panel = () => {
  const live = useStore(navigation.$viewMode) === 'live';
  const selected = useStore(editorModel.$selectedId);
  const placement = useStore(editorModel.$placement);
  const entity = useStore(editorModel.$entity);
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value;
  const baseline = useStore(navigation.$baseline);
  const plan = baseline ? result?.baseline : result?.plan;
  if (!result || !plan) return null;
  if (live) return <Timeline/>;
  const engineer = scenario.engineers.find(engineer => engineer.id === selected);
  if (placement === 'map' || entity !== 'engineer' || !engineer) return null;
  const route = plan.routes.find(route => route.engineerId === selected);
  return <div className="border-t bg-white p-5 max-h-[44vh] overflow-auto" data-testid="engineer-details">
    <div className="flex items-center justify-between gap-4 mb-4"><h2 className="text-lg font-medium">Почему этот маршрут? · {engineer.name}</h2><Changes/></div>
    <div className="flex flex-wrap gap-3 text-sm">
      <Reason icon={<GraduationCap size={20}/>} title="Подходят навыки" text={engineer.skills.map(skill => skillNames[skill]).join(', ')}/>
      <Reason icon={<Clock size={20}/>} title="Временные окна" text={route?.stops.length ? `${route.stops.length} заявок в пределах доступных окон` : 'Назначенных заявок пока нет'}/>
      <Reason icon={<Car size={20}/>} title={transportNames[engineer.transport]} text={`Смена ${formatTime(engineer.shiftStart, engineer.shiftEnd)}`}/>
    </div>
    <h3 className="font-medium mt-5 mb-3">Маршрут и расписание</h3>
    <div className="flex min-w-max pb-4 overflow-x-auto">
      <StopCard icon={<Car size={18}/>} title="Старт" subtitle="База" time={clock(minutes(route?.stops[0]?.departure || engineer.shiftStart))}/>
      {route?.stops.map((stop, i) => <StopCard key={stop.jobId} icon={i + 1} title={stop.jobId} subtitle={scenario.jobs.find(job => job.id === stop.jobId)?.address || ''} time={formatTime(stop.start, stop.end)} explanation={String(stop.explanation)} onClick={() => editorModel.startEditJob(stop.jobId)}/>)}
      <StopCard icon={<Flag size={18}/>} title="Финиш" subtitle={route?.stops.length ? 'Последняя заявка' : 'База'} time={clock(minutes(route?.stops.at(-1)?.end || engineer.shiftStart))}/>
    </div>
    <div className="flex flex-wrap gap-8 border rounded-xl p-3 mt-3 text-sm">
      <span><MapPin size={16} className="inline mr-2"/>Пробег <b>{((route?.distance || 0) / 1000).toFixed(1)} км</b></span>
      <span><Clock size={16} className="inline mr-2"/>В пути <b>{duration(route?.stops.reduce((sum, stop) => sum + (+stop.arrival - +stop.departure) / 60000, 0) || 0)}</b></span>
      <span>Обслуживание <b>{duration(route?.stops.reduce((sum, stop) => sum + (+stop.end - +stop.start) / 60000, 0) || 0)}</b></span>
    </div>
  </div>;
};
function duration(value: number) { return `${Math.floor(value / 60)} ч ${Math.round(value % 60)} мин`; }
function Reason({ icon, title, text }: { icon: preact.ComponentChildren; title: string; text: string }) {
  return <div className="flex gap-3 border rounded-xl p-3 max-w-72"><span className="rounded-full bg-yellow-300 size-10 shrink-0 grid place-items-center">{icon}</span><div><b>{title}</b><p className="text-gray-500 mt-1">{text}</p></div></div>;
}
function StopCard({ icon, title, subtitle, time, explanation, onClick }: { icon: preact.ComponentChildren; title: string; subtitle: string; time: string; explanation?: string; onClick?: () => void }) {
  return <button className="relative flex-1 min-w-36 max-w-60 text-center px-3" onClick={onClick} title={explanation}>
    <div className="absolute top-4 left-0 right-0 h-[2px] bg-yellow-300"/>
    <span className="relative mx-auto rounded-full bg-yellow-300 size-9 grid place-items-center font-bold mb-2">{icon}</span>
    <b className="text-sm">{title}</b><div className="text-xs text-gray-500 max-w-48 whitespace-normal">{subtitle}</div><div className="text-sm mt-1">{time}</div>
  </button>;
}
function Changes() {
  const [open, setOpen] = useState(false);
  const result = useStore(navigation.$result).value;
  return <details open={open} onToggle={e => setOpen(e.currentTarget.open)} className="text-sm max-w-sm"><summary>Изменения после перепланирования</summary><ul className="mt-2 text-gray-500">{result?.changes.length ? result.changes.map((change, i) => <li key={i}>{change}</li>) : <li>Перепланирования ещё не было</li>}</ul></details>;
}
