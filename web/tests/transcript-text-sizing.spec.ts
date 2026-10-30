import { test, expect, type BrowserContext } from '@playwright/test';
import { mkdir, writeFile } from 'node:fs/promises';
import { control } from './session';
let cookies:Parameters<BrowserContext['addCookies']>[0]=[];
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];});
test('WEB-MSG-TEXT-SIZING the exact reported string stays on one line inside its bubble',async({page,context},info)=>{
    await context.addCookies(cookies);await page.goto('/__fixtures/text-sizing');
    const content=page.locator('[data-message="text-0"] .message-text');await expect(content).toBeVisible();
    const result=await content.evaluate(element=>{const range=document.createRange();range.selectNodeContents(element);const bubble=element.closest('.bubble')!,rect=range.getBoundingClientRect(),box=bubble.getBoundingClientRect(),style=getComputedStyle(element),bubbleStyle=getComputedStyle(bubble),clone=document.createElement('span');for(const property of ['font-family','font-size','font-weight','letter-spacing','font-feature-settings','font-variation-settings'])clone.style.setProperty(property,style.getPropertyValue(property));Object.assign(clone.style,{position:'fixed',left:'-10000px',whiteSpace:'pre',width:'max-content'});clone.textContent=element.textContent;document.body.append(clone);const intrinsic=clone.getBoundingClientRect().width;clone.remove();const context=document.createElement('canvas').getContext('2d')!;context.font=style.fontStyle+' '+style.fontWeight+' '+style.fontSize+' '+style.fontFamily;context.letterSpacing=style.letterSpacing;return {lines:new Set(Array.from(range.getClientRects()).map(rect=>rect.top)).size,overflow:rect.bottom-box.bottom,dom_intrinsic_width:intrinsic,canvas_width:context.measureText(element.textContent??'').width,allocated_inner_width:box.width-parseFloat(bubbleStyle.paddingLeft)-parseFloat(bubbleStyle.paddingRight),font:style.font,letter_spacing:style.letterSpacing};});
    await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/private/web/transcript-text-sizing-exact-${info.project.name}.json`,JSON.stringify(result,null,2));await page.screenshot({path:`../lab/evidence/private/web/transcript-text-sizing-exact-${info.project.name}.png`});
    expect(result.lines).toBe(1);expect(result.overflow).toBeLessThanOrEqual(.5);
});
test('WEB-MSG-TEXT-SIZING 500 seeded mixed-script strings stay inside their bubbles',async({page,context},info)=>{
    await context.addCookies(cookies);await page.goto('/__fixtures/text-sizing');
    await page.evaluate(async()=>{await document.fonts.load('13px InterVariable');await document.fonts.ready;});
    const viewport=page.getByRole('listbox',{name:'Text sizing transcript'});await expect(viewport).toBeVisible();
    const result=await viewport.evaluate(async element=>{
        const seen=new Set<string>(),categories=new Set<string>(),failures=new Map<string,unknown>();
        const settle=()=>new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
        element.scrollTop=0;await settle();
        for(let step=0;step<1500;step++){
            for(const bubble of element.querySelectorAll<HTMLElement>('.bubble:not(.media-reserved)')){
                const id=bubble.closest<HTMLElement>('[data-message]')!.dataset.message!,box=bubble.getBoundingClientRect(),walker=document.createTreeWalker(bubble.querySelector('.message-text')!,NodeFilter.SHOW_TEXT);
                seen.add(id);const value=bubble.querySelector('.message-text')!.textContent??'';
                if(/[A-Za-z]{16}/u.test(value))categories.add('long_word');if(/\p{Extended_Pictographic}/u.test(value))categories.add('emoji');if(/\p{Script=Han}/u.test(value))categories.add('cjk');if(/[\p{Script=Arabic}\p{Script=Hebrew}]/u.test(value))categories.add('rtl');if(/[A-Za-z]/u.test(value)&&/[\p{Script=Han}\p{Script=Arabic}\p{Script=Hebrew}]/u.test(value))categories.add('mixed');
                while(walker.nextNode()){
                    const range=document.createRange();range.selectNodeContents(walker.currentNode);
                    for(const rect of range.getClientRects())if(rect.width>0&&(rect.left<box.left-.5||rect.right>box.right+.5||rect.top<box.top-.5||rect.bottom>box.bottom+.5))failures.set(id,{id,text:bubble.textContent,box:box.toJSON(),rect:rect.toJSON(),font:getComputedStyle(bubble).font});
                }
            }
            if(seen.size===501)break;
            element.scrollTop+=element.clientHeight*.75;await settle();
        }
        return {seen:seen.size,categories:[...categories].sort(),failures:[...failures.values()]};
    });
    await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/private/web/transcript-text-sizing-${info.project.name}.json`,JSON.stringify(result,null,2));
    expect(result.seen).toBe(501);expect(result.categories).toEqual(['cjk','emoji','long_word','mixed','rtl']);expect(result.failures).toEqual([]);
});
test('WEB-MSG-TEXT-SIZING every loaded demo text bubble contains its DOM ranges',async({page,context,request},info)=>{
    await page.route('**/*?transcript-font-probe=1',async route=>{await new Promise(resolve=>setTimeout(resolve,100));await route.continue();});
    await context.addCookies(cookies);const ids=await control(request,'/demo-fixtures'),records=new Map<string,any>();
    page.on('websocket',socket=>socket.on('framereceived',({payload})=>{const frame=JSON.parse(String(payload));for(const item of [...frame.data?.items??[],...(frame.changes??[]).flatMap((change:any)=>change.row?[change.row]:change.data?.items??[])])if(item.collection==='messages.message')records.set(item.id,item);}));
    await page.goto('/messages/'+ids['message-chat']+'/'+ids['demo-message-00-000000']);
    const viewport=page.getByRole('listbox',{name:'Messages transcript'});await expect(viewport).toBeVisible();
    await page.evaluate(async()=>{await document.fonts.ready;await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));});
    const result=await viewport.evaluate(async element=>{
        const seen=new Set<string>(),failures:string[]=[],widths:Record<string,number>={};element.scrollTop=0;
        for(let step=0;step<180;step++){
            await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
            for(const bubble of element.querySelectorAll<HTMLElement>('.bubble:not(.media-reserved)')){
                const content=bubble.querySelector('.message-text');if(!content)continue;seen.add(bubble.closest<HTMLElement>('[data-message]')!.dataset.message!);
                const box=bubble.getBoundingClientRect(),walker=document.createTreeWalker(bubble,NodeFilter.SHOW_TEXT);widths[bubble.closest<HTMLElement>('[data-message]')!.dataset.message+':'+bubble.dataset.part]=box.width;
                while(walker.nextNode()){if(walker.currentNode.parentElement?.closest('.sticker-layer'))continue;const range=document.createRange();range.selectNodeContents(walker.currentNode);for(const rect of range.getClientRects())if(rect.width&& (rect.left<box.left-.5||rect.right>box.right+.5||rect.top<box.top-.5||rect.bottom>box.bottom+.5))failures.push(bubble.textContent??'');}
            }
            if(failures.length||element.scrollTop+element.clientHeight>=element.scrollHeight-1)break;element.scrollTop+=element.clientHeight*.75;
        }
        return {seen:seen.size,failures,widths};
    });
    const expected=[...records.values()].flatMap(item=>{const data=item.data;if(data.kind==='group_event'||data.retracted_at)return [];const parts=data.parts?.length?data.parts:data.text?[{index:0,kind:'text'}]:[];return parts.flatMap((part:any,index:number)=>['text','mention'].includes(part.kind)&&!(data.retracted_parts??[]).includes(part.index??index)?[item.id+':'+(part.index??index)]:[]);}).sort();
    expect(result.failures).toEqual([]);expect(result.seen).toBeGreaterThan(100);expect(Object.keys(result.widths).sort()).toEqual(expected);
    const loaded=await page.locator('.messages-transcript').evaluate(async(element,previous)=>{
        let source='';for(const sheet of Array.from(document.styleSheets))for(const rule of Array.from(sheet.cssRules))if(rule instanceof CSSFontFaceRule){const raw=rule.style.getPropertyValue('src').match(/url\(([^)]+)\)/u)?.[1]?.replace(/["']/gu,'');if(raw){source=new URL(raw,sheet.href??location.href).href;break;}}
        let events=0;const listener=()=>events++;document.fonts.addEventListener('loadingdone',listener);
        const fresh=new URL(source);fresh.searchParams.set('transcript-font-probe','1');const face=new FontFace('TranscriptLateFont',`url("${fresh.href}")`,{weight:'100 900'});const states=[{face:face.status,set:document.fonts.status}];document.fonts.add(face);
        const viewport=element.querySelector<HTMLElement>('.virtual-viewport')!;viewport.scrollTop=viewport.scrollHeight*.4;await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));const top=viewport.getBoundingClientRect().top,anchor=Array.from(viewport.querySelectorAll<HTMLElement>('[data-message]')).find(row=>row.getBoundingClientRect().bottom>top)!;const anchorBefore={id:anchor.dataset.message!,offset:anchor.getBoundingClientRect().top-top};
        const node=element as HTMLElement;node.style.fontFamily='TranscriptLateFont,monospace';node.style.fontFeatureSettings='"kern" 0, "liga" 0';node.style.fontVariationSettings='"wght" 650';
        const pending=document.fonts.load('650 13px TranscriptLateFont');states.push({face:face.status,set:document.fonts.status});await pending;await face.loaded;await document.fonts.ready;await new Promise<void>(resolve=>{const observer=new ResizeObserver(()=>{observer.disconnect();resolve();});observer.observe(element.querySelector('.text-font-probe')!);});
        await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve()))));
        document.fonts.removeEventListener('loadingdone',listener);
        const anchorAfter={id:anchor.dataset.message!,offset:anchor.getBoundingClientRect().top-viewport.getBoundingClientRect().top};const failures:string[]=[],widths:Record<string,number>={};viewport.scrollTop=0;
        for(let step=0;step<180;step++){
            await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
            for(const bubble of element.querySelectorAll<HTMLElement>('.bubble:not(.media-reserved)')){const content=bubble.querySelector('.message-text');if(!content)continue;const box=bubble.getBoundingClientRect(),walker=document.createTreeWalker(bubble,NodeFilter.SHOW_TEXT);widths[bubble.closest<HTMLElement>('[data-message]')!.dataset.message+':'+bubble.dataset.part]=box.width;while(walker.nextNode()){if(walker.currentNode.parentElement?.closest('.sticker-layer'))continue;const range=document.createRange();range.selectNodeContents(walker.currentNode);for(const rect of range.getClientRects())if(rect.width&&(rect.left<box.left-.5||rect.right>box.right+.5||rect.top<box.top-.5||rect.bottom>box.bottom+.5))failures.push(bubble.textContent??'');}}
            if(failures.length||viewport.scrollTop+viewport.clientHeight>=viewport.scrollHeight-1)break;viewport.scrollTop+=viewport.clientHeight*.75;
        }
        return {events,states,anchorBefore,anchorAfter,status:face.status,failures,widths,width_changes:Object.keys(widths).filter(key=>previous[key]!==undefined&&Math.abs(widths[key]-previous[key])>1)};
    },result.widths);
    await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/private/web/transcript-text-sizing-font-${info.project.name}.json`,JSON.stringify(loaded,null,2));
    expect(loaded.status).toBe('loaded');expect(loaded.states.some(state=>state.face==='loading'&&state.set==='loading')).toBe(true);expect(loaded.anchorAfter.id).toBe(loaded.anchorBefore.id);expect(Math.abs(loaded.anchorAfter.offset-loaded.anchorBefore.offset)).toBeLessThanOrEqual(.5);expect(loaded.failures).toEqual([]);expect(Object.keys(loaded.widths).sort()).toEqual(expected);expect(loaded.width_changes.length).toBeGreaterThan(0);
});
test('WEB-MSG-TEXT-SIZING DOM height corrections preserve the visible scroll anchor',async({page,context})=>{
    await context.addCookies(cookies);await page.goto('/__fixtures/text-sizing');
    const viewport=page.getByRole('listbox',{name:'Text sizing transcript'});await expect(viewport).toBeVisible();
    await viewport.evaluate(element=>element.scrollTop=2000);
    const before=await viewport.evaluate(async element=>{
        await new Promise<void>(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve())));
        const top=element.getBoundingClientRect().top,row=Array.from(element.querySelectorAll<HTMLElement>('[data-message]')).find(row=>row.getBoundingClientRect().bottom>top)!;
        return {id:row.dataset.message!,offset:row.getBoundingClientRect().top-top};
    });
    await page.getByRole('button',{name:'Change shaping'}).click();
    const selected=page.locator('[data-message="'+before.id+'"]');
    await expect.poll(()=>selected.evaluate(element=>element.getBoundingClientRect().top-element.closest('.virtual-viewport')!.getBoundingClientRect().top)).toBeCloseTo(before.offset,0);
    await expect.poll(()=>selected.locator('.bubble').evaluate(element=>{
        const range=document.createRange();range.selectNodeContents(element.querySelector('.message-text')!);return range.getBoundingClientRect().bottom-element.getBoundingClientRect().bottom;
    })).toBeLessThanOrEqual(.5);
});
