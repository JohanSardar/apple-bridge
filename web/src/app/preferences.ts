import { createSignal } from 'solid-js';
export type Theme = 'system' | 'light' | 'dark';
const system = matchMedia('(prefers-color-scheme: dark)');
let saved: string | null = null;
try {
    saved = localStorage.getItem('ab.appearance');
}
catch {
    saved = null;
}
const parsed: {
    theme?: Theme;
    textSize?: number;
} = (() => { try {
    return JSON.parse(saved ?? '{}');
}
catch {
    return {};
} })();
const [theme, setTheme] = createSignal<Theme>(['system', 'light', 'dark'].includes(parsed.theme ?? '') ? parsed.theme! : 'system');
const [textSize, setTextSize] = createSignal(Math.min(1.5, Math.max(.85, typeof parsed.textSize==='number'&&Number.isFinite(parsed.textSize)?parsed.textSize:1)));
let persistent = false;
function apply() {
    document.documentElement.dataset.theme = theme() === 'system' ? system.matches ? 'dark' : 'light' : theme();
    document.documentElement.style.setProperty('--text-size', String(textSize()));
    document.querySelector('meta[name=theme-color]')?.setAttribute('content',document.documentElement.dataset.theme==='dark'?(matchMedia('(max-width:699px)').matches?'#000000':'#1e1e1e'):'#f2f2f7');
}
function save() {
    apply();
    if (persistent)
        try {
            localStorage.setItem('ab.appearance', JSON.stringify({ theme: theme(), textSize: textSize() }));
        }
        catch {
            return;
        }
}
export function setPreferencePersistence(value: boolean) { persistent = value; if (!value) {
    try {
        localStorage.removeItem('ab.appearance');
    }
    catch {
        return;
    }
} }
export const appearance = { theme, textSize, setTheme(value: Theme) { setTheme(value); save(); }, setTextSize(value: number) { setTextSize(value); save(); } };
system.addEventListener('change', apply);
apply();
const device = navigator as Navigator & {
    deviceMemory?: number;
};
if (!CSS.supports('backdrop-filter', 'blur(1px)') || (device.deviceMemory !== undefined && device.deviceMemory <= 2) || navigator.hardwareConcurrency <= 2)
    document.documentElement.classList.add('solid-material');
