import { useStore } from "@nanostores/preact";
import classNames from "classnames"
import { ClipboardClock, LoaderCircle, Road, UsersRound } from "lucide-preact"
import type { ComponentChild, FunctionalComponent } from "preact"
import { $tab } from "./model";
import { navigation } from "../../entity/navigation";
import { DASH } from "../../shared/const";

export const TabSwitch = () => {
    const tab = useStore($tab);
    const scenario = useStore(navigation.$scenario).value;
    const busy = useStore(navigation.$busy);
    const resultAsync = useStore(navigation.$result);
    const baseline = useStore(navigation.$baseline);
    const plan = baseline ? resultAsync.value?.baseline : resultAsync.value?.plan;

    return <div className={'grid grid-cols-3 gap-2 my-2'}>
        <Panel
            icon={<UsersRound size={16} />}
            count={scenario.engineers.length}
            countUsed={plan?.metrics.engineers}
            title={'инженеров'}
            selected={tab === 'worker'}
            onClick={() => $tab.set('worker')}

        />
        <Panel
            icon={<ClipboardClock size={16} />}
            count={scenario.jobs.length}
            countUsed={plan ? scenario.jobs.length - plan.metrics.unassigned : undefined}
            title={'заявок'}
            selected={tab === 'job'}
            onClick={() => $tab.set('job')}
        />
        <Panel
            icon={<Road size={16} />}
            count={<>
                {!busy && !resultAsync.value && DASH}
                { busy && <LoaderCircle className={'animate-spin text-primary'} />}
                { !busy && resultAsync.value !== null && `${Math.round(resultAsync.value.stats.elapsed_ms / 100)/ 10} сек`}
            </>}
            title={'метрики'}
            selected={tab === 'metrics'}
            onClick={() => $tab.set('metrics')}
        />
    </div>
}

// Before the search we display only count.
// After a route has been found we display number in a form (assigned/total).
type PanelProps = {
    title: string,
    icon: ComponentChild,
    count: ComponentChild,
    countUsed?: ComponentChild,
    selected?: boolean;
    onClick?: () => void;
}
const Panel: FunctionalComponent<PanelProps> = ({title, icon, count, countUsed, selected, onClick}) => {
    return <button type="button" className={classNames('outline flex flex-col gap-0.5 rounded-lg p-2', selected ? 'outline-yellow-300 outline-2 hover:bg-amber-50' : 'outline-gray-200', selected === false && 'hover:outline-black hover:bg-gray-50', selected !== undefined ? 'cursor-pointer' : 'outline-transparent border border-dashed dash border-gray-300')} onClick={onClick}>
        <div className={'flex gap-2 items-center'}>
            {icon}
            <div className={"font-medium text-lg flex gap-1 h-7 items-center"}>
                {countUsed !== undefined && <div className={'text-yellow-500'}>
                    {countUsed}
                </div>}
                {countUsed !== undefined && <div className={'text-gray-400'}>
                    /
                </div>}
                <div className={classNames('font-medium')}>
                    {count}
                </div>
            </div>
        </div>
        <div className={'text-gray-500 text-sm text-center'}>
            {title}
        </div>
    </button>
}
