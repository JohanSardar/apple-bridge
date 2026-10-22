import {test,expect,type BrowserContext} from '@playwright/test';
import {control} from './session';
let cookies:Parameters<BrowserContext['addCookies']>[0]=[],ids:Record<string,string>;
test.beforeAll(async({request})=>{cookies=[(await control(request,'/session')).cookie];ids=await control(request,'/demo-fixtures');});
test('WEB-MSG-SEND-VARIANTS ordinary send proof does not expose unproven native variants',async({page,context})=>{
    await context.addCookies(cookies);await page.goto('/messages/'+encodeURIComponent(ids['message-chat']));await expect(page.locator('.messages-transcript').first()).toHaveAttribute('aria-busy','false');const response=await page.request.get('/web/v1/sources');expect(response.status()).toBe(200);const body=await response.json(),source=body.records.find((record:any)=>record.source==='messages');expect(source).toBeTruthy();const operations=source.data.coverage.operations,capabilities=source.data.capabilities;
    for(const operation of ['send_later','send_effect','send_formatted','send_mention','send_audio','keep_audio']){const name='messages.'+operation;expect(operations.find((proof:any)=>proof.operation===name)).toMatchObject({proven:false,reason:'per_os_variant_proof_required'});expect(capabilities).not.toContain(name);}
    const field=page.getByRole('textbox',{name:'Message',exact:true});await field.fill('[ab-test] hidden unproven variants');await expect(page.getByRole('button',{name:'Send message',exact:true})).toBeVisible();for(const label of ['Send Later','Send with effect','Format message','Mention a member','Record audio message'])await expect(page.getByRole('button',{name:label,exact:true})).toHaveCount(0);await field.fill('');
});
