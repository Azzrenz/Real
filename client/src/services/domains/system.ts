// System-level actions that hand a path back to the OS.

import { http } from '../http';
import { EP } from '../endpoints';

export const systemApi = {
  /** Open a path in the OS file manager: a file gets selected, a directory just opens. */
  reveal: (path: string) => http.post<{ ok: boolean; path: string }>(EP.reveal(), { path }),
};
