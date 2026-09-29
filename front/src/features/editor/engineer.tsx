import { useStore } from '@nanostores/preact';
import { editorModel } from './model';
import { navigation } from '../../entity/navigation';
import { Layout } from './common';
import { Coordinates, Field, TimeInput } from './fields';
import { skillNames, transportNames } from '../../widgets/settings/engineers';
import type { Skill, Transport } from '../../entity/navigation/worker';
import { Button } from '../../shared/ui/button';
export const EngineerEditor = () => {
  const engineer = useStore(editorModel.$currentEngineer);
  const valid = useStore(editorModel.$isValid);
  const mode = useStore(editorModel.$mode);
  const disabled = useStore(navigation.$busy) || useStore(navigation.$historyLocked);
  return <Layout title={mode === 'edit' ? engineer.name : 'Новый инженер'} action="Сохранить" canSave={valid && !disabled} onSave={editorModel.save} onClose={editorModel.close}>
    <fieldset disabled={disabled} className="flex flex-col gap-3 pr-3">
      <Field label="ID"><input className="control" value={engineer.id} disabled={mode === 'edit'} onInput={e => editorModel.$currentEngineer.setKey('id', e.currentTarget.value)}/></Field>
      <Field label="Имя"><input className="control" value={engineer.name} onInput={e => editorModel.$currentEngineer.setKey('name', e.currentTarget.value)}/></Field>
      <Field label="Транспорт"><select className="control" value={engineer.transport} onChange={e => editorModel.$currentEngineer.setKey('transport', e.currentTarget.value as Transport)}>{Object.entries(transportNames).map(([key, title]) => <option value={key} key={key}>{title}</option>)}</select></Field>
      <Field label="База инженера"><Coordinates value={engineer.start} onChange={value => editorModel.$currentEngineer.setKey('start', value)}/></Field>
      <TimeInput label="Начало смены" value={engineer.shiftStart} onChange={value => editorModel.$currentEngineer.setKey('shiftStart', value)}/>
      <TimeInput label="Конец смены" value={engineer.shiftEnd} onChange={value => editorModel.$currentEngineer.setKey('shiftEnd', value)}/>
      <Field label="Навыки">{Object.entries(skillNames).map(([key, title]) => <label className="flex gap-2" key={key}><input type="checkbox" checked={engineer.skills.includes(key as Skill)} onChange={e => editorModel.$currentEngineer.setKey('skills', e.currentTarget.checked ? [...engineer.skills, key as Skill] : engineer.skills.filter(skill => skill !== key))}/>{title}</label>)}</Field>
      {mode === 'edit' && <Button variant="outline" onClick={editorModel.remove}>Удалить инженера</Button>}
    </fieldset>
  </Layout>;
};
