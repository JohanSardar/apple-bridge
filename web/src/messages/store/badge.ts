import { strings as s } from '../../app/strings';
import { createEffect, onCleanup, type Accessor } from 'solid-js';
export function trackUnread(unread: Accessor<number>) {
    const existing=document.querySelector<HTMLLinkElement>('link[rel="icon"]');
    const icon=existing??document.createElement('link');
    if(!existing){icon.rel='icon';icon.href='/icon.svg';document.head.append(icon);}
    const href=icon?.getAttribute('href');
    const badge=navigator as Navigator & {setAppBadge?:(count:number)=>Promise<void>;clearAppBadge?:()=>Promise<void>};
    createEffect(()=>{
        const count=unread();
        document.title=count?`(${count}) ${s.messages}`:s.messages;
        void (count?badge.setAppBadge?.(count):badge.clearAppBadge?.())?.catch(()=>{});
        if(icon)icon.href=count?'data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="16" fill="#34c759"/><path fill="white" d="M51 30c0 11-12 19-28 16l-10 5 3-10C2 21 29 7 46 18c3 3 5 7 5 12"/><circle cx="52" cy="12" r="11" fill="#0088ff" stroke="white" stroke-width="3"/></svg>'):href??'/icon.svg';
    });
    onCleanup(()=>{document.title=s.messages;if(!existing)icon.remove();else if(href)icon.setAttribute('href',href);void badge.clearAppBadge?.().catch(()=>{});});
}
