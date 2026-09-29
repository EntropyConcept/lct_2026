import { useStore } from '@nanostores/preact';
import { ArrowDown, ArrowUp, ChartNoAxesCombined, Check, CheckCheck, ChevronDown, CircleAlert, Clock3, LoaderCircle, Route, ShieldCheck, UsersRound, Zap } from 'lucide-preact';
import type { ComponentChildren } from 'preact';
import { navigation } from '../../entity/navigation';
import type { PlanMetrics } from '../../entity/navigation/types';
import './metrics.css';

const number = (value: number, digits = 0) => value.toLocaleString('ru-RU', { maximumFractionDigits: digits });
const distance = (metres: number) => `${number(metres / 1000, 1)} км`;
const seconds = (ms: number) => `${number(ms / 1000, 2)} с`;
const criteria: Record<string, string> = {
  urgent_unassigned: 'Срочные без назначения', unassigned: 'Без назначения',
  engineers: 'Инженеров в плане', distance_m: 'Общий пробег',
};
const comparison: { key: keyof PlanMetrics; label: string; format: (value: number) => string }[] = [
  { key: 'urgentUnassigned', label: 'Срочные без назначения', format: number },
  { key: 'unassigned', label: 'Без назначения', format: number },
  { key: 'engineers', label: 'Инженеров', format: number },
  { key: 'distance', label: 'Пробег', format: distance },
];

export const Metrics = () => {
  const result = useStore(navigation.$result).value;
  const scenario = useStore(navigation.$scenario).value;
  const baseline = useStore(navigation.$baseline);
  const busy = useStore(navigation.$busy);
  const status = useStore(navigation.$status).value;
  const loadingText = status === 'routing' ? 'Подготавливаем дороги' : status === 'rerouting' ? 'Перестраиваем план' : 'Рассчитываем план';

  if (!result) return <section className="plan-metrics metrics-empty" aria-label="Метрики плана" aria-busy={busy}>
    <span className="metrics-empty-icon">{busy ? <LoaderCircle size={26} className="animate-spin"/> : <ChartNoAxesCombined size={26}/>}</span>
    <h2>{busy ? loadingText : 'Здесь будет результат'}</h2>
    <p>{busy ? 'Собираем маршруты и проверяем назначения. Метрики появятся после расчёта.' : 'Загрузите заявки и инженеров, затем нажмите «Построить маршрут», чтобы оценить план.'}</p>
  </section>;

  const plan = baseline ? result.baseline : result.plan;
  const { metrics } = plan;
  const snapshot = result.scenario ?? scenario;
  const total = snapshot.jobs.length;
  const assigned = Math.max(0, total - metrics.unassigned);
  const coverage = total ? Math.min(100, assigned / total * 100) : 0;
  const urgentTotal = snapshot.jobs.filter(job => job.urgent).length;
  const stats = result.stats;

  return <section className="plan-metrics" aria-label="Метрики плана" aria-busy={busy}>
    <div className="metrics-heading">
      <div><span className="metrics-eyebrow">РЕЗУЛЬТАТ РАСЧЁТА</span><h2>Метрики плана</h2></div>
      <span className="metrics-ready">{busy ? <LoaderCircle size={12} className="animate-spin"/> : <Check size={12}/>}{busy ? 'Обновление' : 'Готово'}</span>
    </div>
    {busy && <p className="metrics-updating" role="status"><LoaderCircle size={14} className="animate-spin"/>{loadingText}. Показан предыдущий результат.</p>}
    <div className="metrics-plan-switch" role="group" aria-label="Отображаемый план">
      <button type="button" aria-pressed={!baseline} onClick={() => navigation.$baseline.set(false)}>Рассчитанный</button>
      <button type="button" aria-pressed={baseline} onClick={() => navigation.$baseline.set(true)}>Базовый</button>
    </div>
    <div className="metrics-coverage">
      <div className="metrics-coverage-top"><span><CheckCheck size={16}/>Назначено заявок</span><span>{total ? `${number(coverage, 1)}%` : '—'}</span></div>
      <div className="metrics-coverage-value"><strong>{number(assigned)}</strong><span>из {number(total)}</span></div>
      <div className="metrics-progress" role="progressbar" aria-label="Доля назначенных заявок" aria-valuenow={coverage} aria-valuemin={0} aria-valuemax={100} aria-valuetext={`${assigned} из ${total}`}><span style={{ width: `${coverage}%` }}/></div>
      <p>{!total ? 'В сценарии пока нет заявок' : metrics.unassigned ? `${number(metrics.unassigned)} без назначения — проверьте ограничения` : 'Все заявки включены в маршруты'}</p>
    </div>
    <div className="metrics-cards">
      <MetricCard icon={<Route size={16}/>} label="Общий пробег" value={number(metrics.distance / 1000, 1)} unit="км" hint="По всем маршрутам"/>
      <MetricCard icon={<UsersRound size={16}/>} label="Инженеров в плане" value={number(metrics.engineers)} hint={`Из ${number(snapshot.engineers.length)} в сценарии`}/>
      <MetricCard icon={<CircleAlert size={16}/>} label="Без назначения" value={number(metrics.unassigned)} hint={metrics.unassigned ? 'Заявки вне маршрутов' : 'Все заявки распределены'} tone={metrics.unassigned ? 'warning' : 'success'}/>
      <MetricCard icon={<Zap size={16}/>} label="Срочные без назначения" value={number(metrics.urgentUnassigned)} hint={urgentTotal ? `Всего срочных: ${number(urgentTotal)}` : 'Срочных заявок нет'} tone={metrics.urgentUnassigned ? 'danger' : 'success'}/>
    </div>
    <section className="metrics-comparison" aria-labelledby="metrics-comparison-title">
      <div className="metrics-section-heading"><h3 id="metrics-comparison-title">Сравнение с базовым</h3><ChartNoAxesCombined size={15}/></div>
      <p className="metrics-caption">Изменения рассчитанного плана относительно базового.</p>
      <table>
        <thead><tr><th scope="col">Показатель</th><th scope="col">База</th><th scope="col">План</th><th scope="col">Разница</th></tr></thead>
        <tbody>{comparison.map(({ key, label, format }) => {
          const before = result.baseline.metrics[key];
          const after = result.plan.metrics[key];
          const delta = after - before;
          return <tr key={key}><th scope="row">{label}</th><td>{format(before)}</td><td>{format(after)}</td><td>
            <span className={`metrics-delta ${delta < 0 ? 'better' : delta > 0 ? 'worse' : ''}`} aria-label={delta ? `${delta < 0 ? 'Меньше' : 'Больше'} на ${format(Math.abs(delta))}` : 'Без изменений'}>
              {delta < 0 ? <ArrowDown size={11}/> : delta > 0 ? <ArrowUp size={11}/> : null}{delta ? format(Math.abs(delta)) : '—'}
            </span>
          </td></tr>;
        })}</tbody>
      </table>
    </section>
    <div className={`metrics-proof ${!baseline && stats.optimal ? 'proven' : ''}`}>
      {baseline ? <Route size={18}/> : stats.optimal ? <ShieldCheck size={18}/> : <Clock3 size={18}/>}
      <div><h3>{baseline ? 'Базовый план' : stats.optimal ? 'Оптимальность доказана' : 'Допустимый план найден'}</h3>
        <p>{baseline ? 'Исходный план для сравнения. Его маршруты и показатели сейчас отображаются на экране.' : stats.scope === 'candidate_routes' ? 'Поиск ограничен набором маршрутов-кандидатов. Глобальная оптимальность не доказана.' : stats.optimal ? 'Оптимум подтверждён по всем критериям модели.' : 'План прошёл проверку ограничений. Оптимальность пока не доказана.'}</p>
      </div>
    </div>
    <details className="metrics-details">
      <summary><span><Clock3 size={15}/>Подробности расчёта</span><span>{seconds(stats.elapsed_ms)}<ChevronDown size={14}/></span></summary>
      <div className="metrics-details-body">
        <p className="metrics-caption">Статистика рассчитанного плана</p>
        <dl>
          <dt>Общее время</dt><dd>{seconds(stats.elapsed_ms)}</dd>
          <dt>Подготовка решения</dt><dd>{seconds(stats.generation_ms)}</dd>
          <dt>Построение SAT-модели</dt><dd>{seconds(stats.encoding_ms)}</dd>
          <dt>Вызовы SAT</dt><dd>{number(stats.sat_calls)}</dd>
          <dt>Переменные / ограничения</dt><dd>{number(stats.variables)} / {number(stats.clauses)}</dd>
          {stats.candidate_routes !== undefined && stats.scope === 'candidate_routes' && <><dt>Маршруты-кандидаты</dt><dd>{number(stats.candidate_routes)}</dd></>}
        </dl>
        {!!stats.stages?.length && <div className="metrics-stages"><h3>Проверка критериев</h3><p className="metrics-caption">Нижняя и верхняя границы в порядке приоритета. Галочка — оптимум критерия доказан.</p>{stats.stages.map(stage => <div className="metrics-stage" key={stage.criterion}>
          <span>{stage.proven ? <Check size={13} aria-label="Доказан"/> : <Clock3 size={13} aria-label="Не доказан"/>}{criteria[stage.criterion] ?? stage.criterion}</span>
          <b>{number(stage.lower)}{stage.lower !== stage.upper && ` – ${number(stage.upper)}`}{stage.criterion === 'distance_m' && ' м'}</b>
        </div>)}</div>}
        {stats.status && <p className="metrics-raw-status">{stats.status}</p>}
      </div>
    </details>
  </section>;
};

function MetricCard({ icon, label, value, unit, hint, tone = '' }: { icon: ComponentChildren; label: string; value: string; unit?: string; hint: string; tone?: string }) {
  return <div className={`metrics-card ${tone}`}><div className="metrics-card-label">{icon}<span>{label}</span></div><div className="metrics-card-value">{value}{unit && <span>{unit}</span>}</div><p>{hint}</p></div>;
}
