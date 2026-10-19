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
  try {
    palette.show(); flush(); await Promise.resolve(); flush();
    const input = root.querySelector('input')!;
    input.value = 'manifest'; input.dispatchEvent(new Event('input', {bubbles: true})); flush();
    await new Promise(resolve => setTimeout(resolve, 220)); flush();
    const file = [...root.querySelectorAll('[role=option]')].find(row => row.textContent?.includes('manifest 上海.pdf'));
    if (!file) throw new Error('File result not rendered');
    if (!root.textContent?.includes('Search covers downloaded history only')) throw new Error('Missing incomplete-history notice');
    input.dispatchEvent(new KeyboardEvent('keydown', {key: 'ArrowDown', bubbles: true})); flush();
    const selected = document.getElementById(input.getAttribute('aria-activedescendant')!);
    if (!selected || selected.getAttribute('aria-selected') !== 'true') throw new Error('Keyboard active option mismatch');
    input.dispatchEvent(new KeyboardEvent('keydown', {key: 'Escape', bubbles: true})); flush();
    if (root.querySelector('[role=dialog]')) throw new Error('Escape did not dismiss');
    return {file: file.textContent, keyboard: 'selected option linked to input', escape: 'dismissed', incomplete: 'visible'};
  } finally { store.searchChats = original; palette.close(); dispose(); root.remove(); }
}
