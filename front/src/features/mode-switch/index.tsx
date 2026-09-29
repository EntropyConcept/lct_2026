import { useStore } from '@nanostores/preact';
import { CirclePlay, Route } from 'lucide-preact';
import { navigation } from '../../entity/navigation';
import { editorModel } from '../editor/model';
export const ModeSwitch = () => {
  const mode = useStore(navigation.$viewMode);
  const result = useStore(navigation.$result).value;
  return <div className="map-mode-switch" role="group" aria-label="Режим карты">
    <button type="button" aria-pressed={mode === 'plan'} onClick={() => navigation.$viewMode.set('plan')}><Route size={15}/>План</button>
    <button type="button" data-testid="live-mode" aria-pressed={mode === 'live'} disabled={!result} onClick={() => { editorModel.close(); navigation.$baseline.set(false); navigation.$viewMode.set('live'); }}><CirclePlay size={15}/>Live</button>
  </div>;
};
