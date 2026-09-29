import { useState } from 'preact/hooks';
import { Calendar } from '../../shared/ui/calendar';
import { ru } from 'date-fns/locale';
import { formatSmartDate } from '../settings/title';
import { CalendarDays, Settings as SettingsIcon } from 'lucide-preact';
import { useStore } from '@nanostores/preact';
import {CircleCheck, CircleDashed, CircleDashedCheck, CircleFadingArrowUp, CircleSmall} from 'lucide-preact';
import { navigation, type Status } from '../../entity/navigation';
import classNames from 'classnames';
import { SettingsDialog } from '../settings/dialog';
import type { ComponentChild } from 'preact';

const statusColorMap: Record<Status, string> = {
    initial: 'text-gray-500',
    routing: 'text-yellow-500',
    search: 'text-yellow-500',
    rerouting: 'text-yellow-500',
    suboptimal: 'text-blue-500',
    optimal: 'text-green-500',
}
const statusTitleMap: Record<Status, string> = {
    initial: 'новый план',
    routing: 'построение графа',
    search: 'оптимизация пути',
    rerouting: 'перерасчет',
    suboptimal: 'первичный план',
    optimal: 'оптимальный план'
}
const statusIconMap: Record<Status, ComponentChild> = {
    initial: <CircleSmall size={16} />,
    routing: <CircleDashed size={16} />,
    search: <CircleDashed size={16} />,
    rerouting: <CircleFadingArrowUp size={16}/>,
    suboptimal: <CircleDashedCheck size={16}/>,
    optimal: <CircleCheck size={16}/>
}

export const Header = () => {
    const date = useStore(navigation.$date);
    const busy = useStore(navigation.$busy);
    const dates = useStore(navigation.$dates);
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [calendarOpen, setCalendarOpen] = useState(false);
    const statusAsync = useStore(navigation.$status);

    return <header className={'min-h-16 shrink-0 px-6 w-full flex justify-between items-center border-b-gray-200 border-b-[1px]'}>
        <div className={'font-medium text-2xl flex items-center gap-2'}>
            <img className={'h-7 w-7'} src="/beeline.png" alt=""></img>
            <span>билайн</span>
            <div className="w-[2px] mx-4 bg-gray-300 h-8"/>
            <span>beekeeper</span>
        </div>
        <div className={'flex gap-4 items-center'}>
            <button aria-label="Настройки" title="Настройки" data-testid="settings-open" className="rounded-lg p-2 hover:bg-gray-100" onClick={event => { event.currentTarget.focus(); setCalendarOpen(false); setSettingsOpen(true); }}><SettingsIcon size={20}/></button>
            <div className="relative">
                <button className="flex gap-2 items-center text-sm" data-testid="date-picker" disabled={busy} onClick={() => setCalendarOpen(!calendarOpen)} aria-expanded={calendarOpen}><CalendarDays size={18}/>{formatSmartDate(date)}</button>
                {calendarOpen && <div className="absolute top-10 right-0 z-[100] rounded-xl border bg-white p-2 shadow-xl">
                    <Calendar mode="single" locale={ru} selected={new Date(date)} defaultMonth={new Date(date)} modifiers={{ hasData: dates.map(value => new Date(value)) }} modifiersClassNames={{ hasData: 'font-bold underline' }} onSelect={value => { if (value) navigation.setDate(value); setCalendarOpen(false); }}/>
                </div>}
            </div>
            {statusAsync.state === 'ready' && statusAsync.value &&
                // fixed width prevents ui jump on update
                <div className={'w-32'}>
                    <div className={classNames('px-2 py-1 w-fit border border-gray-200 rounded-full font-medium flex items-center gap-2 whitespace-nowrap', statusColorMap[statusAsync.value])}>
                        {statusIconMap[statusAsync.value]}
                        {statusTitleMap[statusAsync.value]}
                    </div>
                </div>
            }
        </div>
        <div className={'hidden lg:flex items-center gap-4'}>
            <div className={'rounded-full bg-gray-200 w-10 h-10 grid place-items-center'}>
                NS
            </div>

            <div className={'flex flex-col'}>
                <span className={'text-base font-medium h-5'}>Назар С.</span>
                <span className={'text-sm text-gray-500'}>
                    Разработчик
                </span>
            </div>
        </div>

        {settingsOpen && <SettingsDialog onClose={() => setSettingsOpen(false)}/>}
    </header>
}
