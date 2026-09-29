import { useStore } from '@nanostores/preact';
import { navigation } from '../../entity/navigation';
import { TabSection } from './tab';
export const Metrics = () => {
  const result = useStore(navigation.$result).value;
  const baseline = useStore(navigation.$baseline);
  const busy = useStore(navigation.$busy);
  return <TabSection title="Информация о решении"><div className="text-sm">
    {busy ? <p>Идёт расчёт…</p> : !result ? <p>Запустите поиск, чтобы увидеть метрики</p> : <>
      <dl className="metrics-grid">
        <dt>Неназначенные</dt><dd>{result.plan.metrics.unassigned}</dd>
        <dt>Срочные неназначенные</dt><dd>{result.plan.metrics.urgentUnassigned}</dd>
        <dt>Инженеров</dt><dd>{result.plan.metrics.engineers}</dd>
        <dt>Пробег</dt><dd>{(result.plan.metrics.distance / 1000).toFixed(1)} км</dd>
        <dt>Расчёт</dt><dd>{(result.stats.elapsed_ms / 1000).toFixed(2)} с</dd>
        <dt>Вызовы SAT</dt><dd>{result.stats.sat_calls}</dd>
      </dl><p className="mt-4">{result.stats.optimal ? 'Оптимальность доказана' : 'Найден допустимый план; оптимальность не доказана'}</p>
      <p className="text-gray-500 mt-2">{result.stats.scope === 'candidate_routes' ? 'Доказательство ограничено набором маршрутов-кандидатов.' : result.stats.scope}</p>
      <p className="mt-2">{result.stats.status}</p>
      <label className="flex gap-2 mt-4"><input type="checkbox" checked={baseline} onChange={e => navigation.$baseline.set(e.currentTarget.checked)}/>Показать базовый план</label>
    </>}
  </div></TabSection>;
};
