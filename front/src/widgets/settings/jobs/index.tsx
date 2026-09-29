import { useStore } from '@nanostores/preact';
import { Plus } from 'lucide-preact';
import { navigation, getAssignedJobs } from '../../../entity/navigation';
import { editorModel } from '../../../features/editor/model';
import { Button } from '../../../shared/ui/button';
import { InfoButtonWrapper, TabSection } from '../tab';
import { JobsTable } from './table';

export const Jobs = () => {
  const scenario = useStore(navigation.$scenario).value;
  const result = useStore(navigation.$result).value;
  const busy = useStore(navigation.$busy);
  const locked = useStore(navigation.$historyLocked);
  const live = useStore(navigation.$viewMode) === 'live';
  const unassigned = result ? getAssignedJobs(scenario, result).filter(job => !job.assigned) : [];
  return <>
    <TabSection title={`Заявки · ${scenario.jobs.length}`} info={<InfoButtonWrapper>
      <Button variant="secondary" size="sm" disabled={busy || (locked && !live)} onClick={editorModel.startNewJob}><Plus size={16}/>{live && result ? 'Срочная заявка' : 'Добавить заявку'}</Button>
    </InfoButtonWrapper>}>
      <JobsTable scenario={scenario} result={result} loading={busy}/>
    </TabSection>
    {result && <TabSection title={`Неназначенные · ${unassigned.length}`}>
      <div className="flex flex-col gap-2">{unassigned.map(job => <button key={job.id} className="text-left rounded-lg border border-amber-200 p-3" onClick={() => editorModel.startEditJob(job.id)}><b>{job.id}</b><p className="text-sm text-gray-500">{job.reason}</p></button>)}{!unassigned.length && <p className="text-sm text-gray-500">Все заявки назначены</p>}</div>
    </TabSection>}
  </>;
};
