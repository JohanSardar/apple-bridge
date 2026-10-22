import { expect } from '@playwright/test';
import { test } from './list-session';
import { control } from './session';
import { probeSearchRender } from './search-render';
test.use({serviceWorkers:'block'});
test('WEB-PERF-SEARCH bounded results render within 50 ms',async({page,request})=>{
    await control(request,'/search-fixture');await probeSearchRender(page);await page.goto('/messages/search?q=orch');
    const rows=page.locator('.search-message'),input=page.getByRole('searchbox',{name:'Search messages',exact:true});await expect(rows).toHaveCount(4);
    for(const query of ['emile','reference.bin','orch','emile','reference.bin','orch','emile']){await input.fill(query);await expect(rows).toHaveCount(query==='orch'?4:1);await expect(rows.first()).toContainText(query==='emile'?'Émile':query==='reference.bin'?'reference.bin':'Orchard');}
    await expect.poll(()=>page.evaluate(()=>(window as any).__searchRender.dom.length)).toBe(8);
    const samples=await page.evaluate(()=>(window as any).__searchRender.dom as number[]);samples.sort((a,b)=>a-b);expect(samples[Math.ceil(samples.length*.95)-1]).toBeLessThanOrEqual(50);
});
