import {test,expect} from '@playwright/test';
import {control} from './session';
test('WEB-MSG-LINKS-SOLID compact previews keep a legible icon on low-power material fallback',async({page,context,request},info)=>{
 const session=await control(request,'/session?kind=public');await context.addCookies([session.cookie]);const ids=await control(request,'/demo-fixtures');
 await page.addInitScript(()=>Object.defineProperty(navigator,'hardwareConcurrency',{configurable:true,value:2}));
 await page.goto('/__fixtures/media?cards=plain&chat='+encodeURIComponent(ids['message-chat']));await expect(page.locator('html')).toHaveClass(/solid-material/u);
 const badge=page.getByLabel('Video link'),card=page.locator('.message-card').first();await expect(badge).toBeVisible();
 const readability=await badge.evaluate(element=>{
  const style=getComputedStyle(element),cardStyle=getComputedStyle(element.closest('.message-card')!),canvas=document.createElement('canvas');canvas.width=canvas.height=1;const context=canvas.getContext('2d',{willReadFrequently:true})!;
  context.fillStyle=cardStyle.backgroundColor;context.fillRect(0,0,1,1);context.fillStyle=style.backgroundColor;context.fillRect(0,0,1,1);const background=Array.from(context.getImageData(0,0,1,1).data).slice(0,3);
  context.fillStyle=style.color;context.fillRect(0,0,1,1);const foreground=Array.from(context.getImageData(0,0,1,1).data).slice(0,3);
  const luminance=(rgb:number[])=>rgb.map(value=>value/255).map(value=>value<=.04045?value/12.92:((value+.055)/1.055)**2.4).reduce((total,value,index)=>total+value*[.2126,.7152,.0722][index],0),a=luminance(foreground),b=luminance(background);
  return {foreground,background,ratio:(Math.max(a,b)+.05)/(Math.min(a,b)+.05),backdrop:style.backdropFilter};
 });
 await info.attach('compact-fallback-contrast',{body:JSON.stringify(readability),contentType:'application/json'});await page.screenshot({path:`../lab/evidence/private/web/media-cards-compact-solid-${info.project.name}.png`});
 expect(readability.backdrop).toBe('none');expect(readability.ratio).toBeGreaterThanOrEqual(3);expect(await card.boundingBox()).toMatchObject({width:280,height:88});
});
