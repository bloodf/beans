// Browser consumer regression: run checkSearchResults() through the installed Solid Vite plugin with ?mock=1.
import { render } from '@solidjs/web';
import { flush } from 'solid-js';
import { PaletteHost, palette } from './palette';
import { store } from '../model/store';
export async function checkSearchResults() {
  const root = document.createElement('div'); document.body.append(root);
  const dispose = render(() => <PaletteHost />, root);
  const original = store.searchChats.bind(store);
  const chat = store.chats[0];
  if (!chat) throw new Error('Synthetic demo chat missing');
  store.searchChats = async () => ({chats: [], messages: [], files: [{chat_id: chat.id, message_id: 'synthetic-message', attachment_id: 'synthetic-file', name: 'manifest 上海.pdf', snippet: 'manifest 上海.pdf', created_at: 1}], history_complete: false});
  const originalRelated = store.searchRelated.bind(store);
  const author = {kind: 'you' as const};
  // One meaning match is new; the other repeats the exact file hit and must not show twice.
  store.searchRelated = async () => ({state: 'ready', runner_id: null, related: [
    {kind: 'message' as const, chat_id: chat.id, message_id: 'synthetic-related', snippet: 'shipping notes for Shanghai', author, created_at: 2, score: 0.8},
    {kind: 'file' as const, chat_id: chat.id, message_id: 'synthetic-message', attachment_id: 'synthetic-file', name: 'manifest 上海.pdf', snippet: 'manifest 上海.pdf', author, created_at: 1, score: 0.7},
  ]});
  try {
    palette.show(); flush(); await Promise.resolve(); flush();
    const input = root.querySelector('input')!;
    input.value = 'manifest'; input.dispatchEvent(new Event('input', {bubbles: true})); flush();
    await new Promise(resolve => setTimeout(resolve, 220)); flush();
    const file = [...root.querySelectorAll('[role=option]')].find(row => row.textContent?.includes('manifest 上海.pdf'));
    if (!file) throw new Error('File result not rendered');
    if (!root.textContent?.includes('Search covers downloaded history only')) throw new Error('Missing incomplete-history notice');
    if (!root.textContent?.includes('Related') || !root.textContent?.includes('shipping notes for Shanghai')) throw new Error('Related result not rendered');
    const fileRows = [...root.querySelectorAll('[role=option]')].filter(row => row.textContent?.includes('manifest 上海.pdf'));
    if (fileRows.length !== 1) throw new Error('A related hit repeated an exact one');
    input.dispatchEvent(new KeyboardEvent('keydown', {key: 'ArrowDown', bubbles: true})); flush();
    const selected = document.getElementById(input.getAttribute('aria-activedescendant')!);
    if (!selected || selected.getAttribute('aria-selected') !== 'true') throw new Error('Keyboard active option mismatch');
    input.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true})); flush();
    if (root.querySelector('[role=dialog]')) throw new Error('Escape did not dismiss');
    return {file: file.textContent, keyboard: 'selected option linked to input', escape: 'dismissed', incomplete: 'visible'};
  } finally { store.searchChats = original; store.searchRelated = originalRelated; palette.close(); dispose(); root.remove(); }
}
