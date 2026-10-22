import { test, expect } from '@playwright/test';
import { createServer } from 'vite';
test('WEB-MSG-CHAT-ACTIONS trailing extension reveals, closes and preserves full swipe confirmation', async ({ page }) => {
    const source = `import { render } from 'solid-js/web'; import { SwipeChatRow } from '/src/messages/list/ChatActionSurface.tsx'; let calls=0;const chat={chat:{id:'fixture'},unread_count:0}; render(()=><SwipeChatRow chat={chat} actions={{can:()=>false}} surface={{open:(chat,anchor,confirm)=>document.body.dataset.confirm=String(confirm)}} extraTrailing={(value,anchor,close)=>{document.body.dataset.calls=String(++calls);document.body.dataset.chat=value.chat.id;anchor.dataset.extension='ready';return <button onClick={close}>Extra</button>;}}><a style={{display:'block',height:'70px'}}>Fixture</a></SwipeChatRow>,document.getElementById('root'));`;
    const server = await createServer({ server: { port: 0 }, plugins: [{ name: 'swipe-fixture', resolveId: id => id === '/__swipe.tsx' ? id : undefined, load: id => id === '/__swipe.tsx' ? source : undefined, configureServer(server) { server.middlewares.use('/__fixture', async (_, response) => { response.setHeader('Content-Type', 'text/html'); response.end(await server.transformIndexHtml('/__fixture', '<div id="root" style="width:400px"></div><script type="module" src="/__swipe.tsx"></script>')); }); } }] });
    try {
        await server.listen(); await page.goto(server.resolvedUrls!.local[0] + '__fixture');
        const root = page.locator('.chat-swipe-row'); await expect(root).toBeVisible();
        const swipe = async (dx: number) => { const box = (await root.boundingBox())!; await page.mouse.move(box.x+380,box.y+35); await page.mouse.down(); await page.mouse.move(box.x+380+dx,box.y+35,{steps:12}); await page.mouse.up(); };
        await expect(page.getByRole('button',{name:'Extra'})).toHaveCount(0); await swipe(-80);
        await expect(root).toHaveAttribute('data-extension','ready'); await expect(page.locator('body')).toHaveAttribute('data-chat','fixture'); await expect(page.locator('body')).toHaveAttribute('data-calls','1');
        await expect(root.locator('.chat-swipe-content')).toHaveCSS('transform','matrix(1, 0, 0, 1, -176, 0)');
        await page.getByRole('button',{name:'Extra',exact:true}).click(); await expect(root.locator('.chat-swipe-buttons')).toHaveCount(0);
        await swipe(-340); await expect(page.locator('body')).toHaveAttribute('data-confirm','true'); await expect(root.locator('.chat-swipe-buttons')).toHaveCount(0);
    } finally { await server.close(); }
});
