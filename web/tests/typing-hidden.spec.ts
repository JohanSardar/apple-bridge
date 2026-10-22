import { expect } from '@playwright/test';
import { test } from './list-session';

test('WEB-MSG-TYPING no unproven presence or per-keystroke command is exposed', async ({ page }) => {
    const frames: any[] = [];
    page.on('websocket', socket => socket.on('framesent', frame => frames.push(JSON.parse(String(frame.payload)))));
    await page.goto('/messages');
    await page.locator('.conversation-row').filter({ hasText: 'Weekend Crew' }).click();
    await page.getByRole('textbox', { name: 'Message', exact: true }).fill('[ab-test typing] local draft');
    await expect(page.locator('[data-typing], [data-typing-indicator], .typing-indicator')).toHaveCount(0);
    await expect(page.getByRole('button', { name: /typing indicator/i })).toHaveCount(0);
    await expect(page.getByText(/is typing|are typing/i)).toHaveCount(0);
    expect(frames.filter(frame => frame.t === 'cmd' || String(frame.op ?? '').includes('typing'))).toEqual([]);
});
