import type { ComponentChildren } from 'preact';
import { format } from 'date-fns';
import { atMinute } from '../../entity/navigation/api';
import { navigation } from '../../entity/navigation';
export const Field = ({ label, children }: { label: string; children: ComponentChildren }) => <label className="flex flex-col gap-1 text-sm">{label}{children}</label>;
export const TimeInput = ({ label, value, onChange }: { label: string; value: Date; onChange: (value: Date) => void }) => <Field label={label}><input className="control" type="time" value={format(value, 'HH:mm')} onInput={e => {
  if (e.currentTarget.value) { const [h, m] = e.currentTarget.value.split(':').map(Number); onChange(atMinute(h * 60 + m, navigation.$date.get())); }
}}/></Field>;
export const Coordinates = ({ value, onChange }: { value: { lat: number; lon: number }; onChange: (point: { lat: number; lon: number }) => void }) => <div className="grid grid-cols-2 gap-2">{(['lat', 'lon'] as const).map(key => <Field key={key} label={key === 'lat' ? 'Широта' : 'Долгота'}><input className="control" type="number" step="0.00001" min={key === 'lat' ? -90 : -180} max={key === 'lat' ? 90 : 180} value={value[key]} onInput={e => onChange({ ...value, [key]: e.currentTarget.value === '' ? NaN : Number(e.currentTarget.value) })}/></Field>)}</div>;
