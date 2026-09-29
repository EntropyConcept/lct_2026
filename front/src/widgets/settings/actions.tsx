import { useStore } from '@nanostores/preact';
import { useRef } from 'preact/hooks';
import { ArrowDownToLine, Route } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { editorModel } from '../../features/editor/model';
import { Button } from '../../shared/ui/button';

export const Actions = () => {
  const busy = useStore(navigation.$busy);
  const locked = useStore(navigation.$historyLocked);
  const scenario = useStore(navigation.$scenario).value;
  const error = useStore(navigation.$error);
  const datasets = useStore(navigation.$datasets);
  const needsRouting = useStore(navigation.$needsRouting);
  const status = useStore(navigation.$status).value;
  const file = useRef<HTMLInputElement>(null);
  return <div className="flex flex-col gap-3">
    <div className="flex gap-2">
      <Button data-testid="solve" className="flex-1" disabled={busy || locked || !scenario.jobs.length || !scenario.engineers.length || needsRouting} onClick={() => navigation.startSearch()}>
        <Route size={16}/>{status === 'routing' ? 'Подготовка дорог…' : busy ? 'Загрузка…' : 'Построить маршрут'}
      </Button>
      <Button variant="outline" disabled={busy} onClick={() => file.current?.click()} aria-label="Импорт данных"><ArrowDownToLine size={16}/></Button>
      <input ref={file} type="file" data-testid="import" accept=".json,.csv" hidden onChange={async event => {
        const input = event.currentTarget;
        const selected = input.files?.[0];
        if (selected) {
          if (selected.size > 2_000_000) navigation.$error.set('Размер файла должен быть не больше 2 МБ.');
          else if (await navigation.importFile(selected.name, await selected.text())) { editorModel.close(); }
        }
        input.value = '';
      }}/>
    </div>
    <select aria-label="Загрузить данные" data-testid="dataset" className="control" disabled={busy} value="" onChange={async event => {
      const path = event.currentTarget.value;
      if (path && await navigation.load(path)) { editorModel.close(); }
    }}>
      <option value="">Загрузить данные…</option>
      <option value="/api/demo">Демонстрационный набор</option>
      <option value="/api/city-demo">Городской пример</option>
      {datasets.map((name, index) => <option key={name} value={`/api/dataset/${index}`}>{name}</option>)}
    </select>
    {needsRouting && !busy && <p className="text-sm text-amber-800">Требуется подготовка дорог. Откройте настройки в верхней панели для повторной подготовки или выбора офлайн-схемы.</p>}
    {locked && <p className="text-sm text-gray-500">История зафиксирована. Доступны новые срочные заявки и отмена ещё не начатых выездов.</p>}
    {error && <div role="alert" className="rounded-lg bg-red-50 p-3 text-sm text-red-800">{error}</div>}
  </div>;
};
