import { atom } from "nanostores";

export const $tab = atom<'worker' | 'job' | 'metrics'>('worker');
