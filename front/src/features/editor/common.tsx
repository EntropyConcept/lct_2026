import type { ComponentChildren } from "preact"
import { Button } from "../../shared/ui/button";
import { X } from "lucide-preact";

type Props = {
    title: string,
    action: ComponentChildren,
    children: ComponentChildren,
    canSave: boolean,
    onSave: () => void;
    onClose: () => void;
}

export const Layout = ({title, action, children, canSave, onClose, onSave}: Props) => {
    return <div className={'h-full flex flex-col gap-2'}>
        <div className={'flex justify-between'}>
            <div className={'font-medium text-lg'}>
                {title}
            </div>
            <Button variant={'secondary'} size='icon-sm' onClick={onClose}>
                <X size={16}/>
            </Button>
        </div>
        <div className={'overflow-y-auto flex-1 min-h-0'}>
            {children}
        </div>
        <Button variant={'secondary'} disabled={!canSave} onClick={onSave} data-testid="editor-save">
            {action}
        </Button>
    </div>
}