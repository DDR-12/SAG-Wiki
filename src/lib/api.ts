import { invoke } from '@tauri-apps/api/core';

export const api = <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
 if (import.meta.env.DEV && typeof location !== 'undefined' && new URLSearchParams(location.search).get('preview') === 'models') {
  return import('../preview-api').then(preview => preview.previewApi<T>(command, args));
 }
 return invoke<T>(command, args);
};
