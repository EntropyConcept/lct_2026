import { format } from "date-fns"
import { DASH } from "../../../shared/const"

export const formatTime = (start: Date, end: Date) => {
    return format(start, 'HH:mm') + DASH + format(end, 'HH:mm')
}