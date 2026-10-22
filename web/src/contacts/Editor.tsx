import { createMemo, createSignal, For, Index, onCleanup, Show } from 'solid-js';
import { object, text, labeled, label, type Item } from './store';
import { ContactAvatar } from './Card';
import './editor.css';
const scalars = [['given_name','First name'],['middle_name','Middle name'],['family_name','Last name'],['name_prefix','Prefix'],['name_suffix','Suffix'],['nickname','Nickname'],['previous_family_name','Previous last name'],['organization','Company'],['job_title','Job title'],['department','Department'],['phonetic_given_name','Phonetic first name'],['phonetic_middle_name','Phonetic middle name'],['phonetic_family_name','Phonetic last name'],['phonetic_organization','Phonetic company']] as const;
const kinds = [['phones','Phone'],['emails','Email'],['urls','Website'],['postal_addresses','Address'],['dates','Date'],['relations','Related name'],['social_profiles','Social profile'],['instant_messages','Instant message']] as const;
type Kind = typeof kinds[number][0];
const details: Partial<Record<Kind,readonly (readonly [string,string])[]>> = {
    postal_addresses:[['street','Street'],['sub_locality','Neighborhood'],['city','City'],['sub_administrative_area','County'],['state','State'],['postal_code','Postal code'],['country','Country'],['iso_country_code','Country code']],
    social_profiles:[['username','Username'],['service','Service'],['url','URL'],['user_identifier','User identifier']],
    instant_messages:[['username','Username'],['service','Service']]
};
const initialValue = (kind: Kind) => kind==='dates'?{month:1,day:1}:details[kind]?{}:'';
const editable = [...scalars.map(([key])=>key),...kinds.map(([key])=>key),'birthday','non_gregorian_birthday'];
const standardLabels = ['home','work','mobile','other'];
export function ContactEditor(props: { item?: Item; initial?: Record<string,unknown>; busy: boolean; cancel:()=>void; save:(event:Event,args:Record<string,unknown>)=>void; remove?:()=>void }) {
    const original = props.item?.contact.data ?? {};
    const seed: Record<string,unknown>={};
    for(const key of editable)if(key in original)seed[key]=structuredClone(original[key]);
    Object.assign(seed,props.initial??{});
    const [draft,setDraft]=createSignal(seed), [addKind,setAddKind]=createSignal<Kind>('phones'), [error,setError]=createSignal(''), [loading,setLoading]=createSignal(false), [preview,setPreview]=createSignal('');
    const [customLabels,setCustomLabels]=createSignal(new Set(kinds.flatMap(([kind])=>labeled(seed[kind]).flatMap((entry,index)=>standardLabels.includes(label(entry.label,'other').toLocaleLowerCase())?[]:[kind+':'+index]))));
    let photoGeneration=0, disposed=false;
    onCleanup(()=>{disposed=true;photoGeneration++;});
    const set=(key:string,value:unknown)=>setDraft(before=>({...before,[key]:value}));
    const changeEntry=(kind:Kind,index:number,key:string,value:unknown)=>{
        const entries=labeled(draft()[kind]).map((entry,i)=>i===index?{...entry,[key]:value}:entry);set(kind,entries);
    };
    const changeDetail=(kind:Kind,index:number,key:string,value:unknown)=>{
        const entries=labeled(draft()[kind]);changeEntry(kind,index,'value',{...object(entries[index]?.value),[key]:value});
    };
    const removeEntry=(kind:Kind,index:number)=>{set(kind,labeled(draft()[kind]).filter((_,i)=>i!==index));setCustomLabels(before=>new Set([...before].flatMap(key=>{if(!key.startsWith(kind+':'))return [key];const i=Number(key.split(':')[1]);return i===index?[]:[kind+':'+(i>index?i-1:i)];})));};
    const add=()=>{const kind=addKind(), entries=labeled(draft()[kind]);if(entries.length>=50){setError('A field can contain up to 50 values.');return;}set(kind,[...entries,{label:'_$!<Home>!$_',value:initialValue(kind)}]);};
    const changed = createMemo(()=>{
        const args:Record<string,unknown>={};
        for(const [key,value] of Object.entries(draft()))if(JSON.stringify(value)!==JSON.stringify(original[key]))args[key]=value;
        return args;
    });
    async function choosePhoto(file: File|undefined) {
        if(!file)return;
        const generation=++photoGeneration;setError('');
        if(file.size>8*1024*1024 || !['image/png','image/jpeg','image/webp','image/avif','image/heic','image/heif'].includes(file.type)){setLoading(false);setError('Choose an image smaller than 8 MB.');return;}
        setLoading(true);
        try {
            const bitmap=await createImageBitmap(file);
            try {
                if(bitmap.width*bitmap.height>40_000_000)throw new Error('Image too large');
                const canvas=document.createElement('canvas'), scale=Math.min(1,512/Math.max(bitmap.width,bitmap.height));
                canvas.width=Math.max(2,Math.round(bitmap.width*scale));canvas.height=Math.max(2,Math.round(bitmap.height*scale));
                const context=canvas.getContext('2d');if(!context)throw new Error('Image unavailable');
                context.fillStyle='#fff';context.fillRect(0,0,canvas.width,canvas.height);context.drawImage(bitmap,0,0,canvas.width,canvas.height);
                let data='';
                for(const quality of [.86,.7,.5,.3]){data=canvas.toDataURL('image/jpeg',quality);if(data.length<=96*1024)break;}
                if(data.length>96*1024)throw new Error('Image too large');
                if(disposed || generation!==photoGeneration)return;
                set('image',{base64:data.split(',')[1]});setPreview(data);
                canvas.width=canvas.height=0;
            } finally { bitmap.close(); }
        } catch { if(!disposed && generation===photoGeneration)setError('This image couldn’t be opened. Try a JPEG or PNG.'); }
        finally { if(!disposed && generation===photoGeneration)setLoading(false); }
    }
    function submit(event: Event) {
        setError('');
        const args=changed();
        if(!props.item && ![...scalars.map(([key])=>text(draft()[key])),...['phones','emails'].flatMap(key=>labeled(draft()[key]).map(entry=>text(entry.value)))].some(value=>value.trim())){setError('Add a name, company, phone or email.');return;}
        for(const key of ['birthday','non_gregorian_birthday'])if(args[key]!=null){const value=object(args[key]);if(!validDate(value)){setError('Enter a valid month and day.');return;}}
        for(const entry of labeled(args.dates))if(!validDate(object(entry.value))){setError('Enter a valid month and day.');return;}
        if(new TextEncoder().encode(JSON.stringify(args)).byteLength>160000){setError('This contact is too large to save in one change.');return;}
        if(props.item && Object.keys(args).length===0){props.cancel();return;}
        props.save(event,args);
    }
    const dateFields=(value:()=>Record<string,unknown>,update:(key:string,value:unknown)=>void,prefix:string)=><div class="contact-date-editor"><For each={[['year','Year',1,9999],['month','Month',1,12],['day','Day',1,31]] as const}>{([key,title,min,max])=><label>{title}<input type="number" aria-label={prefix+' '+title.toLowerCase()} min={min} max={key==='month' && value().calendar && value().calendar!=='gregorian'?13:max} value={typeof value()[key]==='number'?String(value()[key]):''} onInput={event=>update(key,event.currentTarget.value===''?undefined:Number(event.currentTarget.value))}/></label>}</For></div>;
    return <section class="contact-editor" aria-label={props.item?'Edit Contact':'New Contact'}>
        <header class="contact-editor-toolbar"><button class="contact-back" disabled={props.busy} onClick={props.cancel}>Cancel</button><h1>{props.item?'Edit Contact':'New Contact'}</h1><button class="contact-back" disabled={props.busy||loading()} onClick={submit}>Save</button></header>
        <div class="contact-photo-editor"><Show when={preview()} fallback={<Show when={props.item && !('image' in changed())}><ContactAvatar item={props.item!} large/></Show>}><img src={preview()} alt="Contact photo preview"/></Show><label class="button subtle">{loading()?'Preparing photo…':'Choose photo'}<input aria-label="Contact photo" type="file" accept="image/png,image/jpeg,image/webp,image/avif,image/heic,image/heif" disabled={props.busy||loading()} onChange={event=>void choosePhoto(event.currentTarget.files?.[0])}/></label><Show when={props.item?.contact.data.image || preview()}><button class="button subtle" type="button" disabled={props.busy} onClick={()=>{photoGeneration++;setLoading(false);set('image',null);setPreview('');}}>Remove photo</button></Show></div><p class="contact-photo-help">Photos are resized to fit your contact card.</p>
        <Show when={error()}><p role="alert">{error()}</p></Show>
        <fieldset disabled={props.busy}>
            <div class="contact-scalar-fields"><For each={scalars}>{([key,title])=><label>{title}<input aria-label={title} autocomplete="off" maxlength="2048" value={text(draft()[key])} onInput={event=>set(key,event.currentTarget.value)}/></label>}</For></div>
            <For each={kinds}>{([kind,title])=><Show when={labeled(draft()[kind]).length}><section class="contact-edit-group"><h2>{title}</h2><Index each={labeled(draft()[kind])}>{(entry,index)=>{
                const normalized=()=>label(entry().label,'other').toLocaleLowerCase();
                return <div class="contact-edit-value"><div class="contact-edit-label"><select aria-label={'Label for '+title+' '+(index+1)} value={customLabels().has(kind+':'+index)?'custom':standardLabels.includes(normalized())?normalized():'custom'} onChange={event=>{const value=event.currentTarget.value;setCustomLabels(before=>{const next=new Set(before);if(value==='custom')next.add(kind+':'+index);else next.delete(kind+':'+index);return next;});changeEntry(kind,index,'label',value==='custom'?'':`_$!<${value[0].toUpperCase()+value.slice(1)}>!$_`);}}><For each={standardLabels}>{value=><option value={value}>{value}</option>}</For><option value="custom">custom</option></select><Show when={customLabels().has(kind+':'+index)||!standardLabels.includes(normalized())}><input aria-label={'Custom label for '+title+' '+(index+1)} value={label(entry().label,'')} maxlength="100" onInput={event=>changeEntry(kind,index,'label',event.currentTarget.value)}/></Show><button type="button" class="contact-remove-field" aria-label={'Remove '+title+' '+(index+1)} onClick={()=>removeEntry(kind,index)}>Remove</button></div>
                    <Show when={kind==='dates'} fallback={<Show when={details[kind]} fallback={<input aria-label={title+' '+(index+1)} maxlength="2048" value={text(entry().value)} type={kind==='emails'?'email':kind==='phones'?'tel':'text'} onInput={event=>changeEntry(kind,index,'value',event.currentTarget.value)}/>}><For each={details[kind]}>{([key,name])=><label>{name}<input aria-label={title+' '+(index+1)+' '+name} maxlength="2048" value={text(object(entry().value)[key])} onInput={event=>changeDetail(kind,index,key,event.currentTarget.value)}/></label>}</For></Show>}>{dateFields(()=>object(entry().value),(key,value)=>changeDetail(kind,index,key,value),title+' '+(index+1))}</Show>
                </div>;
            }}</Index></section></Show>}</For>
            <div class="contact-add-field"><select aria-label="Field type" value={addKind()} onChange={event=>setAddKind(event.currentTarget.value as Kind)}><For each={kinds}>{([key,title])=><option value={key}>{title}</option>}</For></select><button type="button" class="button subtle" onClick={add}>Add field</button></div>
            <For each={[['birthday','Birthday'],['non_gregorian_birthday','Other birthday']] as const}>{([key,title])=><section class="contact-edit-group"><h2>{title}</h2><Show when={draft()[key]!=null} fallback={<button type="button" class="button subtle" onClick={()=>set(key,{month:1,day:1})}>{'Add '+title.toLowerCase()}</button>}>{dateFields(()=>object(draft()[key]),(part,value)=>set(key,{...object(draft()[key]),[part]:value}),title)}<button type="button" class="contact-remove-field" onClick={()=>set(key,null)}>{'Remove '+title.toLowerCase()}</button></Show></section>}</For>
        </fieldset>
        <Show when={props.remove}><button type="button" class="button danger contact-delete" disabled={props.busy} onClick={props.remove}>Delete Contact</button></Show>
    </section>;
}
function validDate(value:Record<string,unknown>) {
    const month=Number(value.month), day=Number(value.day),year=value.year===undefined?2000:Number(value.year);
    if(!Number.isInteger(month)||!Number.isInteger(day)||!Number.isInteger(year)||month<1||month>(value.calendar && value.calendar!=='gregorian'?13:12)||day<1||day>31||year<1||year>9999)return false;
    if(value.calendar && value.calendar!=='gregorian')return true;
    const date=new Date(2000,month-1,day);date.setFullYear(year);
    return date.getMonth()===month-1 && date.getDate()===day;
}
