import type { ComponentChild } from "preact"
import type { PropsWithChildren } from "preact/compat"

export const Tab = ({children}: PropsWithChildren) => {
    return <div className={'flex flex-col gap-4'}>
        {children}
    </div>
}

type SectionProps = {
    children: ComponentChild,
    title: string,
    info?: ComponentChild,
}

export const TabSection = ({children, title, info}: SectionProps) => {
    return <div className={'flex flex-col gap-2 min-h-0'}>
                <div className={'flex gap-2 items-center'}>
                    <span className={'font-medium text-lg whitespace-nowrap'}>
                        {title}
                    </span>
                    {info}
                </div>
                {children}
            </div>
}

export const InfoButtonWrapper = ({children}: PropsWithChildren) => {
    return <div className={'ml-auto flex justify-end'}>
        {children}
    </div>
}
