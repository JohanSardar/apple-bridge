import { strings as s } from '../../app/strings';
import { createSignal, For, onCleanup, onMount, Show } from 'solid-js';
import { Icon } from '../../design/Icon';
import { chatFilters, type ChatFilter } from '../store/chats';
export function FiltersMenu(props:{value:ChatFilter;onChange:(value:ChatFilter)=>void}) {
    const [open,setOpen]=createSignal(false);
    let root!:HTMLDivElement,trigger!:HTMLButtonElement;
    const close=()=>{setOpen(false);trigger.focus();};
    onMount(()=>{
        const outside=(event:PointerEvent)=>{if(!root.contains(event.target as Node))setOpen(false);};
        const escape=(event:KeyboardEvent)=>{if(open()&&event.key==='Escape'){event.preventDefault();close();}};
        document.addEventListener('pointerdown',outside);document.addEventListener('keydown',escape);
        onCleanup(()=>{document.removeEventListener('pointerdown',outside);document.removeEventListener('keydown',escape);});
    });
    return <div ref={root} class="conversation-filters" onKeyDown={event=>{
        if(!open())return;
        if(['ArrowDown','ArrowUp','Home','End'].includes(event.key)){event.preventDefault();const buttons=[...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role=menuitemradio]')];const current=buttons.indexOf(document.activeElement as HTMLButtonElement);const next=event.key==='Home'?0:event.key==='End'?buttons.length-1:(current+(event.key==='ArrowDown'?1:-1)+buttons.length)%buttons.length;buttons[next].focus();}
    }}><button ref={trigger} class="icon-button" aria-label={s.filters} title="Filters" aria-haspopup="menu" aria-expanded={open()} onClick={()=>setOpen(!open())} onKeyDown={event=>{if(event.key==='ArrowDown'){event.preventDefault();setOpen(true);queueMicrotask(()=>root.querySelector<HTMLButtonElement>('[role=menuitemradio]')?.focus());}}}><Icon name="ellipsis"/></button><Show when={open()}><div class="conversation-filter-menu glass" role="menu" aria-label={s.messageFilters}><For each={chatFilters}>{filter=><button role="menuitemradio" aria-checked={props.value===filter.value} onClick={()=>{props.onChange(filter.value);close();}}>{filter.label}<Show when={props.value===filter.value}><Icon name="check"/></Show></button>}</For></div></Show></div>;
}
