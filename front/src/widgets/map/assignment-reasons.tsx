import type { ComponentChildren } from 'preact';
import { ArrowRight, Car, Check, Clock3, GraduationCap, Route, TriangleAlert } from 'lucide-preact';
import { format } from 'date-fns';
import type { Job } from '../../entity/navigation/job';
import type { Engineer } from '../../entity/navigation/worker';
import type { RouteStop } from '../../entity/navigation/types';
import { skillNames, transportNames } from '../settings/engineers';
import { formatTime } from '../settings/jobs/utils';

const duration = (start: Date, end: Date) => Math.max(0, Math.round((+end - +start) / 60000));

export function AssignmentReasons({ job, engineer, stop, baseline }: {
  job: Job; engineer: Engineer; stop: RouteStop; baseline: boolean;
}) {
  const inWindow = stop.start >= job.windowStart && stop.start <= job.windowEnd;
  const inShift = stop.departure >= engineer.shiftStart && stop.end <= engineer.shiftEnd;
  const waiting = duration(stop.arrival, stop.start);

  return <section aria-label="Причины назначения" className="shrink-0 rounded-xl border border-gray-200 overflow-hidden">
    <div className="flex items-center gap-2 border-b border-gray-100 bg-gray-50/80 px-3 py-2.5">
      <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-yellow-300"><Check size={16} aria-hidden="true"/></span>
      <h3 className="font-medium">Почему назначена</h3>
    </div>
    <ul className="flex flex-col gap-3 p-3">
      <Criterion icon={<GraduationCap size={17}/>} matched={engineer.skills.includes(job.skill)}
        title="Навык подходит" fallback="Проверьте навык" detail={skillNames[job.skill]}/>
      <Criterion icon={<Car size={17}/>} matched={!job.transport || job.transport === engineer.transport}
        title="Транспорт подходит" fallback="Проверьте транспорт"
        detail={`${transportNames[engineer.transport]}${job.transport ? ' · по требованию заявки' : ' · ограничений нет'}`}/>
      <Criterion icon={<Clock3 size={17}/>} matched={inWindow}
        title="Начало в нужном окне" fallback="Начало вне окна"
        detail={`${format(stop.start, 'HH:mm')} · окно ${formatTime(job.windowStart, job.windowEnd)}`}/>
      <Criterion icon={<Check size={17}/>} matched={inShift}
        title="Работа в пределах смены" fallback="Работа выходит за смену"
        detail={`Готово в ${format(stop.end, 'HH:mm')} · смена до ${format(engineer.shiftEnd, 'HH:mm')}`}/>
    </ul>
    <dl className="grid grid-cols-2 gap-2 px-3 pb-3">
      <div className="rounded-lg bg-slate-50 p-2.5">
        <dt className="flex items-center gap-1.5 text-xs text-slate-500"><ArrowRight size={13} aria-hidden="true"/>Переезд</dt>
        <dd className="mt-1"><span className="text-lg font-semibold tabular-nums">{duration(stop.departure, stop.arrival)}</span> <span className="text-xs text-slate-500">мин</span>
          <span className="mt-0.5 block text-[11px] text-slate-500">{engineer.transport === 'public' ? 'С ожиданием транспорта' : formatTime(stop.departure, stop.arrival)}</span>
        </dd>
      </div>
      <div className="rounded-lg bg-slate-50 p-2.5">
        <dt className="flex items-center gap-1.5 text-xs text-slate-500"><Clock3 size={13} aria-hidden="true"/>До начала работ</dt>
        <dd className="mt-1"><span className="text-lg font-semibold tabular-nums">{waiting}</span> <span className="text-xs text-slate-500">мин</span>
          <span className="mt-0.5 block text-[11px] text-slate-500">{waiting ? 'Ожидание на месте' : 'Сразу по прибытии'}</span>
        </dd>
      </div>
    </dl>
    <div className="flex gap-2 border-t border-amber-100 bg-amber-50/70 p-3">
      <Route size={16} className="mt-0.5 shrink-0 text-amber-700" aria-hidden="true"/>
      <div className="min-w-0 text-xs leading-relaxed">
        <p className="font-medium text-gray-800">{baseline ? 'Часть базового маршрута' : 'Порядок учитывает весь маршрут'}</p>
        <p className="mt-1 text-gray-600">{baseline ? 'Назначение из базового плана для сравнения.' : 'Цель — выполнить заявки с меньшим числом инженеров и пробегом.'}</p>
        {!baseline && <p className="mt-1 text-gray-500">Возможны другие варианты порядка.</p>}
      </div>
    </div>
  </section>;
}

function Criterion({ icon, matched, title, fallback, detail }: {
  icon: ComponentChildren; matched: boolean; title: string; fallback: string; detail: string;
}) {
  return <li className="flex items-start gap-2.5">
    <span className={`mt-0.5 grid size-7 shrink-0 place-items-center rounded-lg ${matched ? 'bg-emerald-50 text-emerald-700' : 'bg-orange-50 text-orange-700'}`} aria-hidden="true">{matched ? icon : <TriangleAlert size={17}/>}</span>
    <div className="min-w-0 flex-1"><p className="text-xs font-medium leading-5">{matched ? title : fallback}</p><p className="text-xs leading-5 text-gray-500">{detail}</p></div>
    {matched && <Check size={14} className="mt-1 shrink-0 text-emerald-600" aria-hidden="true"/>}
  </li>;
}
