import { atom } from 'nanostores';

// Shared with the map so transport markers only animate during explicit playback.
export const $playing = atom(false);
