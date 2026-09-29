import { columnFilteringFeature, columnVisibilityFeature, createColumnHelper, createFilteredRowModel, createSortedRowModel, filterFn_includesString, rowSelectionFeature, rowSortingFeature, sortFn_alphanumeric, sortFn_text, tableFeatures, useTable } from "@tanstack/preact-table"
import { getAssignedJobs, type AssignedJob, type PlanResult, type Scenario } from "../../../entity/navigation";
import { DASH } from "../../../shared/const";
import { useMemo } from "preact/hooks";
import { cn } from "cn";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "../../../shared/ui/table";
import { formatTime } from "./utils";
import { editorModel } from "../../../features/editor/model";
import { useStore } from "@nanostores/preact";
import type { PropsWithChildren } from "preact/compat";

const features = tableFeatures({
  columnFilteringFeature,
  columnVisibilityFeature,
  rowSelectionFeature,
  rowSortingFeature,
  filteredRowModel: createFilteredRowModel(),
  sortedRowModel: createSortedRowModel(),
  filterFns: { includesString: filterFn_includesString },
  sortFns: { alphanumeric: sortFn_alphanumeric, text: sortFn_text },
});

type DataTableFeatures = typeof features;
type Status = 'searching' | 'new' | 'assigned' | 'unassigned'
type JobWithStatus = AssignedJob & {status: Status};

const columnHelper = createColumnHelper<DataTableFeatures, JobWithStatus>();

const statusTitleMap: Record<Status, string> = {
    new: 'новая',
    searching: 'в поиске',
    assigned: 'назначена',
    unassigned: 'не назначена',
}
const statusColorMap: Record<Status, string> = {
    new: 'bg-gray-400',
    searching: 'bg-yellow-300',
    assigned: 'bg-green-400',
    unassigned: 'bg-orange-400',
}
const StatusBadge = ({status}: {status: Status}) => {
    return <div className={'flex gap-2 items-center'}>
        <div className={cn('w-2 h-2 mt-0.5 rounded-full', statusColorMap[status])}/>
        <div>
            {statusTitleMap[status]}
        </div>
    </div>
}

const DimText = ({children}: PropsWithChildren) => {
    return <div className={'whitespace-nowrap text-sm text-gray-500'}>
        {children}
    </div>
}

const tableColumns = columnHelper.columns([
    columnHelper.accessor('id', {
        header: 'ID',
        cell: ({getValue, row}) => <span>{getValue()}{row.original.urgent && <span className="text-red-600 ml-1" title="Срочная заявка">!</span>}</span>,
    }),
    columnHelper.accessor('address', {
        header: 'Адрес',
    }),
    columnHelper.accessor('status', {
        header: 'Статус',
        cell: ({getValue}) => {
            return <StatusBadge status={getValue()}/>
        }
    }),
    columnHelper.accessor('engineer.name', {
        header: 'Инженер',
        cell: ({getValue}) => getValue() ?? DASH,
    }),
    columnHelper.accessor('stop', {
        header: 'Время',
        cell: ({getValue, row}) => {
            const stop = getValue();
            if (row.original.status === 'assigned' && !!stop) {
                return <DimText>
                    {formatTime(stop.start, stop.end)}
                </DimText>
            }
            return <DimText>
                {formatTime(row.original.windowStart, row.original.windowEnd)}
            </DimText>
        }
    }),
    columnHelper.accessor('duration', {
        header: 'Длительность',
        cell: ({getValue}) => <DimText>
            {`${getValue()} мин`}
        </DimText>
    })
])


type TableProps = {
    scenario: Scenario,
    result?: PlanResult | null,
    loading?: boolean,
}

export const JobsTable = ({scenario, loading = false, result = null}: TableProps) => {
    const jobs = useMemo(():JobWithStatus[] =>  {
        if (result) {
            return getAssignedJobs(scenario, result).map((job): JobWithStatus => ({...job, status: job.assigned ? 'assigned' : 'unassigned'}));
        }
        return scenario.jobs.map((job): JobWithStatus => ({...job, assigned: false, reason: '', status: loading ? 'searching' : 'new'}))
    }, [scenario, loading, result]);

    const table = useTable({
        features,
        columns: tableColumns,
        data: jobs,
    });
    const selectedId = useStore(editorModel.$selectedId);
    const selectedEntity = useStore(editorModel.$entity);
    const mode = useStore(editorModel.$mode);
    const jobSelected = mode === 'edit' && selectedEntity === 'job' && !!selectedId;

    return <Table>
            <TableHeader>
                {table.getHeaderGroups().map((headerGroup) => (
                    <TableRow key={headerGroup.id}>
                    {headerGroup.headers.map((header) => {
                        return (
                        <TableHead key={header.id}>
                            {header.isPlaceholder ? null : (
                            <table.FlexRender header={header} />
                            )}
                        </TableHead>
                        )
                    })}
                    </TableRow>
                ))}
            </TableHeader>
            <TableBody>
            {table.getRowModel().rows?.length ? (
                table.getRowModel().rows.map((row) => (
                <TableRow
                    key={row.id}
                    onClick={() => editorModel.startEditJob(row.original.id)}
                    data-state={row.getIsSelected() && "selected"}
                    className={cn('cursor-pointer', jobSelected && selectedId === row.original.id && 'bg-amber-50')}
                >
                    {row.getVisibleCells().map((cell) => (
                    <TableCell key={cell.id}>
                        <table.FlexRender cell={cell} />
                    </TableCell>
                    ))}
                </TableRow>
                ))
            ) : (
                <TableRow>
                <TableCell colSpan={tableColumns.length} className="h-24 text-center">
                    Пока нет заявок
                </TableCell>
                </TableRow>
            )}
            </TableBody>
        </Table>
}
