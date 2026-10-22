import { strings as s } from '../../app/strings';
import { Show } from 'solid-js';
import { Icon } from '../../design/Icon';
export function SearchField(props:{value:string;onInput:(value:string)=>void;onSearch:(value:string)=>void;ref?:(element:HTMLInputElement)=>void}) {
    let input!:HTMLInputElement;
    return <div class="conversation-search"><Icon name="search"/><input ref={element=>{input=element;props.ref?.(element);}} type="search" aria-label={s.search} placeholder={s.search} autocomplete="off" value={props.value} onInput={event=>props.onInput(event.currentTarget.value)} onKeyDown={event=>{if(event.isComposing)return;if(event.key==='Enter'&&props.value.trim()){event.preventDefault();props.onSearch(props.value.trim());}if(event.key==='Escape'){props.onInput('');input.blur();}}}/><Show when={props.value}><button aria-label={s.clearSearch} onClick={()=>{props.onInput('');input.focus();}}><Icon name="close"/></button></Show></div>;
}
