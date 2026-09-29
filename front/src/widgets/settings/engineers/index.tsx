import { useStore } from '@nanostores/preact';
import { Plus } from 'lucide-preact';
import { navigation, getAssignedEngineers } from '../../../entity/navigation';
import { editorModel } from '../../../features/editor/model';
import { Button } from '../../../shared/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '../../../shared/ui/table';
import { InfoButtonWrapper, TabSection } from '../tab';
import { formatTime } from '../jobs/utils';
export const transportNames = { car: 'Автомобиль', walk: 'Пешком', bicycle: 'Велосипед', public: 'Общ. транспорт' };
export const skillNames = { local: 'Локальные', connection: 'Подключение', emergency: 'Аварийные' };

export const Engineers = () => {
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value;
  const busy = useStore(navigation.$busy);
  const locked = useStore(navigation.$historyLocked);
  const selected = useStore(editorModel.$selectedId);
  const entity = useStore(editorModel.$entity);
  const engineers = result ? getAssignedEngineers(scenario, result) : scenario.engineers.map(engineer => ({ ...engineer, assigned: false, stops: [] }));
  const free = engineers.filter(engineer => !engineer.stops?.length);
  return <>
    <TabSection title={`Инженеры · ${engineers.length}`} info={<InfoButtonWrapper><Button variant="secondary" size="sm" disabled={busy || locked} onClick={editorModel.startNewEngineer}><Plus size={16}/>Добавить</Button></InfoButtonWrapper>}>
      <Table data-testid="engineers"><TableHeader><TableRow><TableHead>Инженер</TableHead><TableHead>Транспорт</TableHead><TableHead>Смена</TableHead><TableHead>Заявок</TableHead><TableHead>Навыки</TableHead></TableRow></TableHeader>
        <TableBody>{engineers.map(engineer => <TableRow key={engineer.id} className={`cursor-pointer ${entity === 'engineer' && selected === engineer.id ? 'bg-amber-50' : ''}`} onClick={() => editorModel.startEditEngineer(engineer.id)}>
          <TableCell><b>{engineer.name}</b><div className="text-xs text-gray-500">{engineer.id}</div></TableCell><TableCell>{transportNames[engineer.transport]}</TableCell><TableCell className="whitespace-nowrap">{formatTime(engineer.shiftStart, engineer.shiftEnd)}</TableCell><TableCell>{busy ? '…' : result ? engineer.stops?.length || 0 : '—'}</TableCell><TableCell>{engineer.skills.map(skill => skillNames[skill]).join(', ')}</TableCell>
        </TableRow>)}{!engineers.length && <TableRow><TableCell colSpan={5} className="h-24 text-center">Пока нет инженеров. Загрузите данные или добавьте исполнителя.</TableCell></TableRow>}</TableBody>
      </Table>
    </TabSection>
    {result && <TabSection title={`Свободные · ${free.length}`}><div className="flex flex-col gap-2">{free.map(engineer => <button className="text-left border rounded-lg p-3" key={engineer.id} onClick={() => editorModel.startEditEngineer(engineer.id)}>{engineer.name}<span className="block text-sm text-gray-500">{transportNames[engineer.transport]} · {formatTime(engineer.shiftStart, engineer.shiftEnd)}</span></button>)}{!free.length && <p className="text-sm text-gray-500">Все инженеры задействованы</p>}</div></TabSection>}
  </>;
};
