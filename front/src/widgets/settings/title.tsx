import { useStore } from "@nanostores/preact";
import {
  differenceInCalendarMonths,
  format,
  isToday,
  isTomorrow,
} from "date-fns";
import {
    ru
} from 'date-fns/locale';
import { navigation } from "../../entity/navigation";

export function formatSmartDate(date: string): string {
  const value = new Date(date);

  if (isToday(value)) {
    return `сегодня, ${format(value, "d MMMM", {locale: ru})}`;
  }

  if (isTomorrow(value)) {
    return `завтра, ${format(value, "d MMMM", {locale: ru})}`;
  }


  const includeYear = differenceInCalendarMonths(value, new Date()) !== 0;

  return `${format(value, "EEEE, d MMMM", {locale: ru})}${includeYear ? ` ${format(value, "yyyy", {locale: ru})}` : ""}`;
}

export const Title = () => {
    const date = useStore(navigation.$date);
    return<div className={'font-medium text-xl'}>
        План на {formatSmartDate(date)}
    </div>
}