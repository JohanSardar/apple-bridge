import {test,expect} from '@playwright/test';
import {mkdir,writeFile} from 'node:fs/promises';
import {prepareSession} from './session';
test('WEB-MOTION WEB-MOTION-PANE-WIDTH uses cached pane geometry during trusted navigation',async({page},info)=>{
    await prepareSession(page);
    await page.addInitScript(()=>{
        const state={active:false,armed:false,reads:[] as string[],transforms:[] as string[]};
        const computed=window.getComputedStyle;window.getComputedStyle=(element,pseudo)=>{if(state.active&&element.matches('.transcript,.phone-details'))state.reads.push('style:'+element.className);return computed.call(window,element,pseudo);};
        const descriptor=Object.getOwnPropertyDescriptor(Element.prototype,'clientWidth')!;
        Object.defineProperty(Element.prototype,'clientWidth',{...descriptor,get(){if(state.active&&this instanceof Element&&this.matches('.transcript,.phone-details'))state.reads.push(this.className);return descriptor.get!.call(this);}});
        window.addEventListener('click',event=>{if((event.target as Element).closest('button[aria-label="New Message"]')){state.active=true;state.armed=true;state.reads=[];state.transforms=[];}},{capture:true});
        window.addEventListener('click',()=>{state.active=false;});
        new MutationObserver(records=>{if(!state.armed)return;for(const record of records){const target=record.target;if(target instanceof HTMLElement&&target.matches('.transcript')&&target.style.transform&&state.transforms.length<200)state.transforms.push(target.style.transform);}}).observe(document,{subtree:true,attributes:true,attributeFilter:['style']});
        (window as any).__paneMotion=state;
    });
    await page.goto('/messages');
    if(process.env.AB_WEB_SERVER==='gateway')await page.getByRole('listbox',{name:'Conversations',exact:true}).waitFor();
    else await page.getByText('No Messages',{exact:true}).waitFor();
    const viewport=page.viewportSize()!,phone=viewport.width<700,results:unknown[]=[];
    for(let phase=0;phase<2;phase++){
        if(phase){await page.setViewportSize({width:viewport.width+24,height:viewport.height});await page.evaluate(()=>{document.documentElement.dir='rtl';});}
        await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const pane=page.locator('main.transcript'),width=await pane.evaluate(element=>element.getBoundingClientRect().width);
        await page.getByRole('button',{name:'New Message',exact:true}).click();await page.getByRole('region',{name:'New message',exact:true}).waitFor();
        await expect.poll(()=>page.evaluate(()=>(window as any).__paneMotion.transforms.length)).toBeGreaterThan(0);
        const result=await page.evaluate(()=>(window as any).__paneMotion as {reads:string[];transforms:string[]});results.push({phase,width,reads:result.reads,first:result.transforms[0]});
        await mkdir('../lab/evidence/private/web',{recursive:true});await writeFile(`../lab/evidence/private/web/pane-motion-${info.project.name}.json`,JSON.stringify(results,null,2));
        expect(result.reads).toEqual([]);
        const match=/^translate([XY])\((-?[\d.]+)px\)$/u.exec(result.transforms[0]);expect(match).not.toBeNull();expect(match![1]).toBe(phone?'X':'Y');
        expect(Number(match![2])).toBeCloseTo(phone?width*(phase?-1:1):8,2);
        await expect(pane).toHaveCSS('transform','none');await page.goBack();await expect(page).toHaveURL(/\/messages$/u);
    }
});
