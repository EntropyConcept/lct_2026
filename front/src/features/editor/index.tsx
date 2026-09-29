import { useStore } from "@nanostores/preact";
import { editorModel } from "./model";
import { JobEditor } from "./job";
import { EngineerEditor } from "./engineer";

export const Editor = () => {
    const mode = useStore(editorModel.$mode);
    const placement = useStore(editorModel.$placement);
    const entity = useStore(editorModel.$entity);

    if (mode === 'none' || placement !== 'sidebar') {
        return null;
    }
    return <div data-testid="sidebar-editor" className={'border-r border-r-gray-200 p-4 w-[300px] shrink-0 min-h-0 flex flex-col gap-4'}>
        {entity === 'job' && <JobEditor/>}
        {entity === 'engineer' && <EngineerEditor/>}
    </div>
}