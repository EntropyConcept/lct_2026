import { useStore } from '@nanostores/preact';
import { useEffect, useRef, useState } from 'preact/hooks';
import { X } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { Button } from '../../shared/ui/button';

export const SettingsDialog = ({ onClose }: { onClose: () => void }) => {
  const dialog = useRef<HTMLDialogElement>(null);
  const busy = useStore(navigation.$busy);
  const locked = useStore(navigation.$historyLocked);
  const scenario = useStore(navigation.$scenario).value;
  const seconds = useStore(navigation.$seconds);
  const solver = useStore(navigation.$solver);
  const needsRouting = useStore(navigation.$needsRouting);
  const status = useStore(navigation.$status).value;
  const norms = useStore(navigation.$norms);
  const error = useStore(navigation.$error);
  const [confirmed, setConfirmed] = useState(false);
  useEffect(() => setConfirmed(false), [scenario]);
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, []);
  return <dialog ref={dialog} className="settings-dialog" aria-labelledby="settings-title" data-testid="settings-dialog" onClose={onClose} onClick={event => { if (event.target === dialog.current) { const rect = dialog.current!.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose(); } }}>
    <div className="flex items-center justify-between border-b pb-4 mb-4">
      <h2 id="settings-title" className="text-xl font-medium">Настройки</h2>
      <button autoFocus aria-label="Закрыть настройки" className="rounded-lg p-2 hover:bg-gray-100" onClick={onClose}><X size={20}/></button>
    </div>
    <div className="flex flex-col gap-5">
    {(scenario.jobs.length > 0 || scenario.engineers.length > 0) && <div className="text-xs text-gray-500">
      <b>{scenario.name}</b>
      {!!scenario.notes?.length && <details className="mt-1"><summary>Примечания к данным</summary>{scenario.notes.map((note, i) => <p key={i} className="mt-1">{note}</p>)}</details>}
    </div>}
    <section>
      <h3 className="font-medium">Параметры расчёта и дороги</h3>
      <fieldset disabled={busy || locked} className="flex flex-col gap-2 pt-3 text-sm">
        <label>Режим <select data-testid="solver-mode" className="control" value={solver} onChange={e => navigation.$solver.set(e.currentTarget.value as 'fast' | 'exact')}><option value="fast">Быстрый SAT</option><option value="exact">Полный SAT</option></select></label>
        <label>Бюджет, секунд <input data-testid="solver-seconds" className="control" type="number" min="0.01" max="300" step="0.1" value={seconds} onInput={e => navigation.$seconds.set(Number(e.currentTarget.value))}/></label>
        <label>Дата транспорта <input className="control" type="date" value={scenario.transit_date || ''} onInput={e => { navigation.setScenario({ ...scenario, transit_date: e.currentTarget.value || undefined }); setConfirmed(false); }}/></label>
        <Button variant="outline" onClick={() => { setConfirmed(false); navigation.geocode(); }}>Найти координаты адресов</Button>
        <label className="flex gap-2"><input type="checkbox" checked={confirmed} onChange={e => setConfirmed(e.currentTarget.checked)}/>Координаты заявок и баз проверены</label>
        <Button variant="outline" onClick={() => navigation.prepare(confirmed)}>Подготовить реальные дороги</Button>
        <Button variant="outline" onClick={navigation.useOffline}>Использовать офлайн-схему</Button>
        <span className="text-gray-500">{status === 'routing' ? 'Подготовка дорог…' : needsRouting ? 'Требуется подготовка дорог' : scenario.routing ? 'Дорожный снимок готов' : 'Офлайн-схема: прямые линии, не дороги'}</span>
        <details><summary>Нормативы обслуживания</summary>{norms.map((norm, index) => <label className="block my-2" key={norm.key}>{norm.name}<input className="control" type="number" min="1" max="1440" value={norm.service} onInput={e => navigation.$norms.set(norms.map((n, i) => i === index ? { ...n, service: Number(e.currentTarget.value) } : n))}/></label>)}
          <Button variant="outline" onClick={() => navigation.setScenario({ ...scenario, jobs: scenario.jobs.map(job => ({ ...job, duration: norms.find(n => n.key === job.jobType)?.service ?? job.duration })) })}>Применить нормативы</Button>
        </details>
      </fieldset>
    </section>
      {error && <p role="alert" className="rounded-lg bg-red-50 p-3 text-sm text-red-800">{error}</p>}
    </div>
  </dialog>;
};
