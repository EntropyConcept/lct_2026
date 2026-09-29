import { atom, computed, map } from 'nanostores';
import type { Engineer } from '../../entity/navigation/worker';
import type { Job } from '../../entity/navigation/job';
import { navigation } from '../../entity/navigation';
import { atMinute } from '../../entity/navigation/api';

const defaultEngineer = (): Engineer => ({ id: '', name: '', transport: 'walk', start: { lat: 55.75, lon: 37.61 },
  shiftStart: atMinute(480, navigation.$date.get()), shiftEnd: atMinute(1080, navigation.$date.get()), skills: ['local'] });
const defaultJob = (): Job => ({ id: '', address: '', point: { lat: 55.75, lon: 37.61 }, duration: 45,
  windowStart: atMinute(navigation.$viewMode.get() === 'live' ? navigation.$time.get() : 540, navigation.$date.get()),
  windowEnd: atMinute(1080, navigation.$date.get()), skill: 'local', jobType: 'local', transport: null, urgent: false });
const validPoint = (point: { lat: number; lon: number }) => Number.isFinite(point.lat) && Number.isFinite(point.lon) && Math.abs(point.lat) <= 90 && Math.abs(point.lon) <= 180;
const nextId = (prefix: string, ids: string[]) => { let i = 1; while (ids.includes(`${prefix}${String(i).padStart(3, '0')}`)) i++; return `${prefix}${String(i).padStart(3, '0')}`; };
const editorModelFactory = () => {
  const $entity = atom<'engineer' | 'job'>('job');
  const $mode = atom<'edit' | 'create' | 'none'>('none');
  const $selectedId = atom('');
  const $placement = atom<'sidebar' | 'map'>('sidebar');
  const $currentEngineer = map<Engineer>(defaultEngineer());
  const $currentJob = map<Job>(defaultJob());
  const $isValid = computed([$currentJob, $currentEngineer, $entity], (job, engineer, entity) => entity === 'engineer'
    ? !!engineer.id.trim() && !!engineer.name.trim() && engineer.shiftEnd > engineer.shiftStart && !!engineer.skills.length && validPoint(engineer.start)
    : !!job.id.trim() && !!job.address.trim() && job.windowEnd >= job.windowStart && Number.isInteger(job.duration) && job.duration > 0 && job.duration <= 1440 && validPoint(job.point));
  const close = () => { $mode.set('none'); $selectedId.set(''); };
  navigation.$date.subscribe(close);
  const save = async () => {
    if (!$isValid.get() || navigation.$busy.get()) return;
    const scenario = navigation.$scenario.get().value;
    const creating = $mode.get() === 'create';
    const entity = $entity.get();
    if (creating && (entity === 'job' ? scenario.jobs : scenario.engineers).some(item => item.id === (entity === 'job' ? $currentJob.get().id : $currentEngineer.get().id))) {
      navigation.$error.set('Этот ID уже существует.'); return;
    }
    if (entity === 'job' && creating && navigation.$viewMode.get() === 'live' && navigation.$result.get().value) {
      if (await navigation.addUrgent({ ...$currentJob.get(), urgent: true })) close();
      return;
    }
    if (navigation.setScenario(entity === 'job' ? { ...scenario, jobs: creating ? [...scenario.jobs, $currentJob.get()] : scenario.jobs.map(job => job.id === $selectedId.get() ? $currentJob.get() : job) }
      : { ...scenario, engineers: creating ? [...scenario.engineers, $currentEngineer.get()] : scenario.engineers.map(engineer => engineer.id === $selectedId.get() ? $currentEngineer.get() : engineer) })) close();
  };
  return {
    $entity, $mode, $selectedId, $placement, $currentEngineer, $currentJob, $isValid, close, save,
    startNewJob: () => {
      if (navigation.$busy.get()) return;
      const scenario = navigation.$scenario.get().value;
      $currentJob.set({ ...defaultJob(), id: nextId('JO', scenario.jobs.map(job => job.id)), point: scenario.engineers[0]?.start || defaultJob().point });
      $placement.set('sidebar');
      $selectedId.set(''); $entity.set('job'); $mode.set('create');
    },
    startNewEngineer: () => {
      if (navigation.$busy.get() || navigation.$historyLocked.get()) return;
      $currentEngineer.set({ ...defaultEngineer(), id: nextId('EN', navigation.$scenario.get().value.engineers.map(engineer => engineer.id)) });
      $placement.set('sidebar');
      $selectedId.set(''); $entity.set('engineer'); $mode.set('create');
    },
    startEditJob: (id: string, placement: 'sidebar' | 'map' = 'sidebar') => {
      const job = navigation.$scenario.get().value.jobs.find(job => job.id === id);
      if (!job) return;
      $placement.set(placement);
      $currentJob.set({ ...job }); $selectedId.set(id); $entity.set('job'); $mode.set('edit');
    },
    startEditEngineer: (id: string, placement: 'sidebar' | 'map' = 'sidebar') => {
      const engineer = navigation.$scenario.get().value.engineers.find(engineer => engineer.id === id);
      if (!engineer) return;
      $placement.set(placement);
      $currentEngineer.set({ ...engineer }); $selectedId.set(id); $entity.set('engineer'); $mode.set('edit');
    },
    remove: () => {
      const scenario = navigation.$scenario.get().value;
      if (navigation.setScenario($entity.get() === 'job' ? { ...scenario, jobs: scenario.jobs.filter(job => job.id !== $selectedId.get()) } : { ...scenario, engineers: scenario.engineers.filter(engineer => engineer.id !== $selectedId.get()) })) close();
    },
  };
};
export const editorModel = editorModelFactory();
