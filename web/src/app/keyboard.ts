type Shortcut = {
    key: string;
    shift?: boolean;
    alt?: boolean;
    action: (event: KeyboardEvent) => void;
    input?: boolean;
};
const shortcuts = new Set<Shortcut>();
export const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
export function registerShortcut(shortcut: Shortcut) { shortcuts.add(shortcut); return () => shortcuts.delete(shortcut); }
window.addEventListener('keydown', event => {
    if (event.isComposing || !(isMac ? event.metaKey : event.ctrlKey) || event[isMac ? 'ctrlKey' : 'metaKey'])
        return;
    const input = event.target instanceof HTMLElement && Boolean(event.target.closest('input,textarea,select,[contenteditable=true]'));
    for (const shortcut of shortcuts) {
        if (event.key.toLowerCase() === shortcut.key.toLowerCase() && Boolean(event.shiftKey) === Boolean(shortcut.shift) && Boolean(event.altKey) === Boolean(shortcut.alt) && (!input || shortcut.input)) {
            event.preventDefault();
            shortcut.action(event);
            return;
        }
    }
});
