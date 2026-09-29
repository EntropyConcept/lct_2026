import { Download } from 'lucide-preact';
import { useStore } from '@nanostores/preact';
import { navigation } from '../../entity/navigation';
import { Button } from '../../shared/ui/button';
export function download(name: string, value: unknown) {
  const url = URL.createObjectURL(new Blob([JSON.stringify(value, null, 2)], { type: 'application/json' }));
  const link = document.createElement('a'); link.href = url; link.download = name; link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
export const Export = () => {
  const result = useStore(navigation.$result).value;
  const busy = useStore(navigation.$busy);
  return <div className="flex gap-2 mt-auto pt-4">
    <Button variant="outline" disabled={busy} onClick={() => download('dispatch-days.json', navigation.exportDays())}><Download size={16}/>Экспорт данных</Button>
    {result && <Button variant="outline" disabled={busy} onClick={() => download('dispatch-plan.json', navigation.exportResult())}>План JSON</Button>}
  </div>;
};
