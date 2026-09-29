import { useStore } from '@nanostores/preact';
import { editorModel } from './model';
import { navigation } from '../../entity/navigation';
import { Layout } from './common';
import { Coordinates, Field, TimeInput } from './fields';
import { skillNames, transportNames } from '../../widgets/settings/engineers';
import type { JobType } from '../../entity/navigation/job';
import type { Skill, Transport } from '../../entity/navigation/worker';
import { Button } from '../../shared/ui/button';
export const JobEditor = () => {
  const job = useStore(editorModel.$currentJob);
  const valid = useStore(editorModel.$isValid);
  const mode = useStore(editorModel.$mode);
  const busy = useStore(navigation.$busy);
  const locked = useStore(navigation.$historyLocked);
  const live = useStore(navigation.$viewMode) === 'live';
  const result = useStore(navigation.$result).value;
  const time = useStore(navigation.$time);
  const urgent = live && !!result && mode === 'create';
  const disabled = busy || (locked && !urgent);
  return <Layout title={urgent ? 'Срочная заявка' : mode === 'edit' ? 'Заявка ' + job.id : 'Новая заявка'} action={urgent ? 'Добавить и пересчитать' : 'Сохранить'} canSave={valid && !disabled} onSave={editorModel.save} onClose={editorModel.close}>
    <fieldset disabled={disabled} className="flex flex-col gap-3 pr-3">
      {urgent && <p className="text-sm text-amber-800">Новая заявка поступает в {String(Math.floor(time / 60)).padStart(2, '0')}:{String(time % 60).padStart(2, '0')}. Начатые выезды сохранятся.</p>}
      <Field label="ID"><input className="control" value={job.id} disabled={mode === 'edit'} onInput={e => editorModel.$currentJob.setKey('id', e.currentTarget.value)}/></Field>
      <Field label="Адрес"><textarea className="control" data-testid="job-address" value={job.address} onInput={e => editorModel.$currentJob.setKey('address', e.currentTarget.value)}/></Field>
      <Coordinates value={job.point} onChange={value => editorModel.$currentJob.setKey('point', value)}/>
      {job.geocodeMatch && <p className="text-xs text-gray-500">Геокодирование: {job.geocodeMatch}</p>}
      <Field label="Тип работы"><select className="control" value={job.jobType} onChange={e => { const type = e.currentTarget.value as JobType; editorModel.$currentJob.setKey('jobType', type); editorModel.$currentJob.setKey('skill', type === 'equipment' ? 'connection' : type); const norm = navigation.$norms.get().find(norm => norm.key === type); if (norm) editorModel.$currentJob.setKey('duration', norm.service); }}><option value="local">Локальная заявка</option><option value="connection">Подключение</option><option value="emergency">Авария</option><option value="equipment">Дозаказ оборудования</option></select></Field>
      <Field label="Навык"><select className="control" value={job.skill} onChange={e => editorModel.$currentJob.setKey('skill', e.currentTarget.value as Skill)}>{Object.entries(skillNames).map(([key, title]) => <option value={key} key={key}>{title}</option>)}</select></Field>
      <Field label="Транспорт"><select className="control" value={job.transport || ''} onChange={e => editorModel.$currentJob.setKey('transport', (e.currentTarget.value || null) as Transport | null)}><option value="">Любой</option>{Object.entries(transportNames).map(([key, title]) => <option value={key} key={key}>{title}</option>)}</select></Field>
      <Field label="Длительность, мин"><input className="control" type="number" min="1" max="1440" value={job.duration} onInput={e => editorModel.$currentJob.setKey('duration', Number(e.currentTarget.value))}/></Field>
      <TimeInput label="Окно с" value={job.windowStart} onChange={value => editorModel.$currentJob.setKey('windowStart', value)}/>
      <TimeInput label="Окно до" value={job.windowEnd} onChange={value => editorModel.$currentJob.setKey('windowEnd', value)}/>
      {!urgent && <label className="flex gap-2 text-sm"><input type="checkbox" checked={job.urgent} onChange={e => editorModel.$currentJob.setKey('urgent', e.currentTarget.checked)}/>Срочная</label>}
      {mode === 'edit' && !locked && <Button variant="outline" onClick={editorModel.remove}>Удалить из набора</Button>}
    </fieldset>
    {mode === 'edit' && live && result && <Button className="mt-4" variant="outline" disabled={busy || result.plan.routes.some(route => route.stops.some(stop => stop.jobId === job.id && stop.departure.getTime() < new Date(navigation.$date.get()).setMinutes(time)))} onClick={async () => { if (await navigation.cancelJob(job.id)) editorModel.close(); }}>Отменить заявку в выбранное время</Button>}
  </Layout>;
};
