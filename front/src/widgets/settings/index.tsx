import { useStore } from "@nanostores/preact";
import { Actions } from "./actions";
import { Jobs } from "./jobs";
import { TabSwitch } from "./tab-switch";
import { Title } from "./title";
import { Workers } from "./workers";
import { $tab } from "./model";
import { Tab } from "./tab";
import { Metrics } from "./metrics";
import { Export } from "./export";

export const Settings = () => {
    const tab = useStore($tab);

    return <div data-testid="settings-sidebar" className={'settings-sidebar border-r border-r-gray-200 p-5 w-[450px] shrink-0 min-h-0 overflow-y-auto flex flex-col gap-4'}>
        <Title/>
        <Actions/>
        <TabSwitch/>
        <Tab>
            {tab === 'job' && <Jobs/>}
            {tab === 'worker' && <Workers/>}
            {tab === 'metrics' && <Metrics/>}
        </Tab>
        <Export/>
    </div>
}