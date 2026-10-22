import { test, expect, type Page } from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { bootstrap, control } from './session';
import { reminderPlate } from './reminders-plates';
test.use({ timezoneId: 'America/New_York', locale: 'en-US', reducedMotion: 'no-preference' });
const rowTitles = (page: Page) => page.locator('.reminder-title');
async function observePanes(page: Page) {
  await page.addInitScript(() => {
    (window as any).__reminderPanes = [];
    const sample = () => {
      const app = document.querySelector<HTMLElement>('.reminders-app'), phase = app?.dataset.phase;
      if (phase === 'push' || phase === 'back') {
        const sidebar = getComputedStyle(app!.querySelector('.reminders-sidebar')!), content = getComputedStyle(app!.querySelector('.reminders-content')!);
        const frames = (window as any).__reminderPanes as unknown[];
        frames.push({ phase, sidebar: sidebar.display, content: content.display, front: Number(content.zIndex), back: Number(sidebar.zIndex), transform: content.transform });
        if (frames.length > 120) frames.shift();
      }
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  });
}
async function reviewScreenshot(page: Page, path: string) {
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await expect(page.locator(':focus-visible')).toHaveCount(0);
  await page.screenshot({ path });
}
async function tracePanes(page: Page) { await page.evaluate(() => { (window as any).__reminderPanes = []; }); }
async function transitionPanes(page: Page, phase: 'push' | 'back') {
  await expect.poll(() => page.evaluate(phase => ((window as any).__reminderPanes ?? []).some((frame: any) => frame.phase === phase && frame.sidebar === 'flex' && frame.content === 'flex' && frame.front > frame.back && frame.transform !== 'none'), phase)).toBe(true);
}
async function openList(page: Page, title: string) {
  const phone = page.viewportSize()!.width < 1024;
  if (await page.getByRole('button', { name: 'Back to lists' }).isVisible()) {
    const previous = decodeURIComponent(new URL(page.url()).pathname.split('/')[2]);
    await tracePanes(page);
    await page.getByRole('button', { name: 'Back to lists' }).click();
    await transitionPanes(page, 'back');
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'home');
    await expect(page.locator('.reminders-content')).toHaveCSS('display', 'none');
    await expect(page.locator('.reminders-content')).toHaveAttribute('aria-hidden', 'true');
    await expect(page.locator('.reminders-sidebar [data-selector]:focus')).toHaveCount(1);
    await expect(page.locator('.reminders-sidebar [data-selector]:focus')).toHaveAttribute('data-selector', previous);
  }
  if (phone) await tracePanes(page);
  await page.getByRole('button', { name: title, exact: true }).click();
  if (phone) await transitionPanes(page, 'push');
  await expect(page.getByRole('heading', { name: title, exact: true })).toBeVisible();
  await expect(page.locator('.reminders-content')).toHaveCSS('transform', 'none');
  if (phone) {
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'list');
    await expect(page.locator('.reminders-sidebar')).toHaveCSS('display', 'none');
    await expect(page.locator('.reminders-sidebar')).toHaveAttribute('aria-hidden', 'true');
    await expect(page.getByRole('heading', { name: 'My Lists', exact: true })).toHaveCount(0);
    await expect(page.getByRole('listbox', { name: 'Reminders', exact: true })).toBeFocused();
  }
}
test('WEB-REM-READ exact demo sidebar, nested sections, completed, sorts, keyboard and native live updates', async ({ page, context, request }, info) => {
  await observePanes(page);
  const subscriptions = new Map<string, string>(); let servedSort = '';
  page.on('websocket', socket => {
    socket.on('framesent', event => { const frame = JSON.parse(String(event.payload)); if (frame.t === 'sub' && frame.view === 'reminders.items') subscriptions.set(frame.id, frame.args.sort ?? 'manual'); });
    socket.on('framereceived', event => { const frame = JSON.parse(String(event.payload)); if (frame.t === 'snap' && subscriptions.has(frame.sub)) servedSort = subscriptions.get(frame.sub)!; });
  });
  const manifest = JSON.parse(await readFile('dist/.vite/manifest.json', 'utf8')), routeChunk = '/' + manifest['src/reminders/Reminders.tsx'].file, requested: string[] = [];
  page.on('request', request => requested.push(new URL(request.url()).pathname));
  await bootstrap(context, request);
  await page.goto('/messages');
  await expect(page.getByRole('button', { name: 'Switch app', exact: true })).toBeVisible();
  expect(requested).not.toContain(routeChunk);
  await page.getByRole('button', { name: 'Switch app', exact: true }).click();
  await page.getByRole('button', { name: 'Reminders', exact: true }).click();
  await expect(page).toHaveURL(/\/reminders$/);
  await expect.poll(() => requested.includes(routeChunk)).toBe(true);
  await expect(page.getByRole('heading', { name: 'My Lists', exact: true })).toBeVisible();
  const pinned = page.getByRole('button', { name: 'Travel', exact: true });
  await expect(pinned).toHaveClass(/reminder-tile/);
  await expect(page.locator('.reminder-tiles button').last()).toHaveAttribute('aria-label', 'Travel');
  await expect(pinned.locator('.reminder-count')).toHaveText('3');
  await expect(pinned.locator('.reminder-tile-glyph')).toHaveText('✈️');
  await expect(pinned.locator('.reminder-tile-glyph')).toHaveCSS('background-color', 'rgb(75, 140, 245)');
  await expect(page.locator('.reminder-pin')).toHaveCount(0);
  await expect(page.locator('.reminder-list-name')).toHaveText(['Home', 'Personal', 'Groceries', 'Projects', 'Flagged errands']);
  for (const [tile, count] of Object.entries({ Scheduled: 4, Flagged: 2, All: 9, Completed: 1, 'Assigned to Me': 1 }))
    await expect(page.getByRole('button', { name: tile, exact: true }).locator('.reminder-count')).toHaveText(String(count));
  await expect(page.getByRole('button', { name: 'Home', exact: true }).locator('.reminder-count')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Flagged errands', exact: true })).toBeDisabled();
  for (const [tag, count] of Object.entries({ home: 2, work: 1, travel: 3, errands: 2 }))
    await expect(page.getByRole('button', { name: '#' + tag, exact: true }).locator('.reminder-count')).toHaveText(String(count));
  await page.getByRole('button', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Personal', exact: true })).toBeHidden();
  await page.getByRole('button', { name: 'Home', exact: true }).press('Enter');
  await expect(page.getByRole('button', { name: 'Personal', exact: true })).toBeVisible();
  await page.locator('.reminders-sidebar-body').evaluate(element => { element.scrollTop = 0; });
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-home-${info.project.name}.png`);
  await page.locator('.reminders-sidebar-body').evaluate(element => { element.scrollTop = element.scrollHeight; });
  if (info.project.use.viewport!.width < 700) {
    const clearance = await page.locator('.reminder-tags').evaluate(element => ({ bottom: element.getBoundingClientRect().bottom, scrollportBottom: element.closest('.reminders-sidebar-body')!.getBoundingClientRect().bottom, switcherTop: document.querySelector('.reminders-sidebar .app-menu-control')!.getBoundingClientRect().top }));
    expect(clearance.bottom).toBeLessThanOrEqual(clearance.switcherTop);
    expect(clearance.scrollportBottom).toBeLessThanOrEqual(clearance.switcherTop);
  }
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-tags-${info.project.name}.png`);
  await page.locator('.reminders-sidebar-body').evaluate(element => { element.scrollTop = 0; });
  await openList(page, 'Travel');
  await expect(rowTitles(page)).toHaveText(['Plan a weekend trip', 'Book a place to stay', 'Compare two quiet cabins']);
  expect(await page.locator('[data-depth]').evaluateAll(rows => rows.map(row => row.getAttribute('data-depth')))).toEqual(['0', '1', '2']);
  const indents = await page.locator('.reminder-ring').evaluateAll(elements => elements.map(element => element.getBoundingClientRect().x));
  expect(indents[1] - indents[0]).toBe(info.project.use.viewport!.width < 1024 ? 22 : 24);
  expect(indents[2] - indents[1]).toBe(info.project.use.viewport!.width < 1024 ? 22 : 24);
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-list-${info.project.name}.png`);
  const list = page.getByRole('listbox', { name: 'Reminders' });
  await page.keyboard.press('Meta+f');
  await expect(page.getByLabel('Search Reminders', { exact: true })).toBeFocused();
  await list.focus(); await list.press('Home'); await list.press('ArrowLeft');
  await expect(rowTitles(page)).toHaveText(['Plan a weekend trip']);
  await list.press('ArrowRight');
  await expect(rowTitles(page)).toHaveText(['Plan a weekend trip', 'Book a place to stay', 'Compare two quiet cabins']);
  await list.press('End');
  await expect(list).toHaveAttribute('aria-activedescendant', /.+/);
  await expect(list.locator('[aria-selected=true]')).toContainText('Compare two quiet cabins');
  const focus = await list.evaluate(el => ({ visible: el.matches(':focus-visible'), outline: getComputedStyle(el).outlineStyle }));
  expect(focus.visible).toBe(true); expect(focus.outline).not.toBe('none');
  await expect(list.locator('[aria-selected=true] .reminder-row')).toHaveCSS('outline-style', 'solid');
  await page.screenshot({ path: `../lab/evidence/private/web/reminders-keyboard-${info.project.name}.png` });
  await page.keyboard.press('Meta+i');
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' })).toContainText('Compare two quiet cabins');
  await page.getByRole('button', { name: 'Close details', exact: true }).click();
  await expect(list).toBeFocused();
  await page.getByRole('button', { name: 'Details for Plan a weekend trip', exact: true }).click();
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' })).toContainText('Plan a weekend trip');
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' }).getByText('High', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Close details', exact: true }).locator('svg')).toHaveCSS('visibility', 'visible');
  await expect.poll(() => page.getByRole('button', { name: 'Close details', exact: true }).locator('svg').evaluate(element => (element as SVGGraphicsElement).getBBox().width)).toBeGreaterThan(0);
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-details-${info.project.name}.png`);
  await page.getByRole('button', { name: 'Close details', exact: true }).click();
  await expect(list).toBeFocused();
  await openList(page, 'Groceries');
  await expect(page.locator('.reminder-section-title')).toHaveText(['Produce', 'Pantry']);
  await expect(rowTitles(page)).toHaveText(['Pick up dinner ingredients', 'Tomatoes and basil', 'Pasta and olive oil']);
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-sections-${info.project.name}.png`);
  await openList(page, 'All');
  await expect(page.locator('.reminder-section-title')).toHaveText(['Personal', 'Groceries', 'Projects', 'Travel']);
  await openList(page, 'Scheduled');
  await expect(page.locator('.reminder-section-title')).toHaveText(['Tuesday, October 20', 'Wednesday, October 21', 'Thursday, October 22', 'Friday, October 23']);
  await openList(page, 'Completed');
  await expect(page.locator('.reminder-section-title')).toHaveText(['Personal']);
  await expect(rowTitles(page)).toHaveText(['Choose the next book']);
  await openList(page, 'Assigned to Me');
  await expect(rowTitles(page)).toHaveText(['Plan a weekend trip']);
  await openList(page, 'Personal');
  await expect(rowTitles(page)).toHaveText(['Take a long walk', 'Water the plants']);
  await page.getByLabel('Show Completed', { exact: true }).check();
  await expect(rowTitles(page)).toHaveText(['Take a long walk', 'Choose the next book', 'Water the plants']);
  await expect(page.locator('.reminder-completed')).toContainText('Choose the next book');
  await expect(page.locator('.reminder-completed .reminder-title')).toHaveCSS('text-decoration-line', 'none');
  const completedStyle = await page.locator('.reminder-completed').evaluate(element => { const ring = getComputedStyle(element.querySelector('.reminder-ring')!); return { fill: ring.backgroundColor, border: ring.borderTopColor, title: getComputedStyle(element.querySelector('.reminder-title')!).color, secondary: getComputedStyle(element.querySelector('.reminder-metadata')!).color }; });
  expect(completedStyle.fill).toBe(completedStyle.border); expect(completedStyle.title).toBe(completedStyle.secondary);
  await reviewScreenshot(page, `../lab/evidence/private/web/reminders-completed-${info.project.name}.png`);
  const tied = await page.locator('.reminder-title').evaluateAll(elements => elements.slice(0, 2).map(element => ({ title: element.textContent!, id: element.closest('[data-key]')!.getAttribute('data-key')! })).sort((a, b) => a.id.localeCompare(b.id)).map(value => value.title));
  await expect(page.locator('.reminders-controls')).toHaveCSS('visibility', 'visible');
  await expect(page.getByLabel('Sort', { exact: true })).toBeVisible();
  for (const sort of ['manual', 'due', 'created', 'priority', 'title']) {
    await page.getByLabel('Sort', { exact: true }).selectOption(sort);
    await expect.poll(() => servedSort).toBe(sort);
    await expect(rowTitles(page)).toHaveText(sort === 'title' ? ['Choose the next book', 'Take a long walk', 'Water the plants'] : sort === 'created' || sort === 'priority' ? [...tied, 'Water the plants'] : ['Take a long walk', 'Choose the next book', 'Water the plants']);
  }
  await openList(page, 'Travel');
  await page.getByLabel('Search Reminders', { exact: true }).fill('quiet cabins');
  await expect(rowTitles(page)).toHaveText(['Compare two quiet cabins']);
  await expect(page.locator('.reminder-title mark')).toHaveText('quiet cabins');
  await page.getByLabel('Search Reminders', { exact: true }).fill('');
  await expect(rowTitles(page)).toHaveText(['Plan a weekend trip', 'Book a place to stay', 'Compare two quiet cabins']);
  const contrast = await page.locator('.reminder-title, .reminder-section-title, .reminder-list-name, .reminder-count, .reminder-notes, .reminder-metadata, .reminder-tag, .reminder-overdue, .reminders-heading h1, .reminder-tile>span:last-of-type').evaluateAll(elements => {
    const rgb = (color: string) => { const values = color.match(/[\d.]+/g)!.map(Number); if (color.startsWith('color(srgb ')) return values.map((value, index) => index < 3 ? value * 255 : value); if (!color.startsWith('rgb')) throw new Error('Unsupported computed color: ' + color); return values; };
    const over = (a: number[], b: number[]) => a.slice(0, 3).map((v, i) => v * (a[3] ?? 1) + b[i] * (1 - (a[3] ?? 1)));
    const lum = (c: number[]) => c.map(v => v / 255).map(v => v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4).reduce((n, v, i) => n + v * [.2126, .7152, .0722][i], 0);
    return elements.filter(el => el.getBoundingClientRect().width > 0).map(el => {
      const ancestors: Element[] = []; for (let node: Element | null = el; node; node = node.parentElement) ancestors.unshift(node);
      let bg = [255, 255, 255]; for (const ancestor of ancestors) bg = over(rgb(getComputedStyle(ancestor).backgroundColor), bg);
      const fg = over(rgb(getComputedStyle(el).color), bg), a = lum(fg), b = lum(bg);
      return { text: el.textContent, ratio: (Math.max(a, b) + .05) / (Math.min(a, b) + .05) };
    });
  });
  expect(contrast.length).toBeGreaterThan(0); for (const value of contrast) expect(value.ratio, value.text ?? '').toBeGreaterThanOrEqual(4.5);
  if (info.project.use.viewport!.width < 1024) {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.getByRole('button', { name: 'Back to lists' }).click();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'home');
    await expect(page.locator('.reminders-content')).toHaveCSS('display', 'none');
    await expect(page.getByRole('button', { name: 'Travel', exact: true })).toBeFocused();
    await page.getByLabel('Search Reminders', { exact: true }).fill('quiet cabins');
    await expect(rowTitles(page)).toHaveText(['Compare two quiet cabins']);
    await expect(page.getByLabel('Search Reminders', { exact: true })).toBeFocused();
    await page.getByLabel('Search Reminders', { exact: true }).fill('');
    await page.getByRole('button', { name: 'Back to lists' }).click();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'home');
    await expect(page.getByRole('button', { name: 'All', exact: true })).toBeFocused();
    await page.getByRole('button', { name: 'Travel', exact: true }).click();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'list');
    await expect(page.locator('.reminders-sidebar')).toHaveCSS('display', 'none');
    await expect(page.getByRole('listbox', { name: 'Reminders', exact: true })).toBeFocused();
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    const y = info.project.use.viewport!.height - 80;
    await page.mouse.move(2, y); await page.mouse.down(); await page.mouse.move(35, y, { steps: 3 });
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'swipe');
    await expect(page.locator('.reminders-sidebar')).toHaveCSS('display', 'flex');
    await page.waitForTimeout(100); await page.mouse.up();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'list');
    await expect(page.locator('.reminders-sidebar')).toHaveCSS('display', 'none');
    await page.mouse.move(2, y); await page.mouse.down(); await page.mouse.move(info.project.use.viewport!.width * .6, y, { steps: 12 });
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'swipe');
    await expect(page.locator('.reminders-sidebar')).toHaveCSS('display', 'flex');
    await expect(page.locator('.reminders-content')).toHaveCSS('display', 'flex');
    await page.mouse.up();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'home');
    await expect(page.locator('.reminders-content')).toHaveCSS('display', 'none');
    await expect(page.getByRole('button', { name: 'Travel', exact: true })).toBeFocused();
    await openList(page, 'Travel');
  }
  const samples: number[] = [];
  for (let index = 0; index < 20; index++) {
    const preserveSidebarFocus = info.project.use.viewport!.width >= 1024 ? ({ 1: 'Travel', 2: 'Flagged', 3: '#travel' } as Record<number, string>)[index] : undefined;
    if (preserveSidebarFocus) await page.getByRole('button', { name: preserveSidebarFocus, exact: true }).focus();
    const changed = await control(request, '/reminders-live?index=' + index);
    await expect(page.locator('.reminder-title').filter({ hasText: changed.title })).toHaveText(changed.title);
    await expect(page.locator('[data-smart=flagged] .reminder-count')).toHaveText(String(1 + Number(changed.flagged)));
    if (preserveSidebarFocus) await expect(page.getByRole('button', { name: preserveSidebarFocus, exact: true })).toBeFocused();
    samples.push(await page.locator('.reminder-title').filter({ hasText: changed.title }).evaluate(() => new Promise<number>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve(Date.now()))))) - changed.started_at_ms);
  }
  samples.sort((a, b) => a - b);
  expect(samples[0]).toBeGreaterThanOrEqual(0); expect(samples[18]).toBeLessThan(1000);
  await mkdir('../lab/evidence/web/metrics', { recursive: true });
  await writeFile(`../lab/evidence/web/metrics/WEB-REM-READ-${info.project.name}.json`, JSON.stringify({ samples: 20, p50_ms: samples[9], p95_ms: samples[18], max_ms: samples[19] }) + '\n');
  await control(request, '/reminders-capacity');
  await expect(page.getByRole('alert')).toContainText('This list is too large to show here yet.');
  await expect(page.getByRole('alert')).toContainText('103 incomplete reminders');
  await expect(page.getByRole('alert')).toContainText('Choose a smaller list or tag.');
  await expect(rowTitles(page)).toHaveCount(0);
  if (info.project.name.startsWith('chromium-')) await reminderPlate(context, info.project.name);
});

test('WEB-REM-READ empty-section keyboard selection and sidebar design', async ({ page, context, request }, info) => {
  await observePanes(page);
  await bootstrap(context, request); await control(request, '/reminders-capacity');
  await page.goto('/reminders');
  await openList(page, 'Bridge Test empty section');
  await expect(page.locator('.reminder-section-title')).toHaveText(['Items', 'Empty']);
  const emptyList = page.getByRole('listbox', { name: 'Reminders', exact: true });
  await emptyList.focus(); await emptyList.press('End');
  await expect(emptyList.locator('[aria-selected=true]')).toContainText('Empty');
  await expect.soft(emptyList.locator('[aria-selected=true] .reminder-section-title')).toHaveCSS('outline-style', 'solid');
  await page.keyboard.press('Meta+i');
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' })).toHaveCount(0);
  await emptyList.press('Enter');
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' })).toHaveCount(0);
  await emptyList.press('Home'); await emptyList.press('Enter');
  await expect(page.getByRole(info.project.use.viewport!.width < 1024 ? 'dialog' : 'complementary', { name: 'Reminder details' })).toContainText('Bridge Test only item');
  await page.getByRole('button', { name: 'Close details', exact: true }).click();
  if (info.project.use.viewport!.width < 1024) {
    await page.getByRole('button', { name: 'Back to lists' }).click();
    await expect(page.locator('.reminders-app')).toHaveAttribute('data-phase', 'home');
  }
  const travel = page.getByRole('button', { name: 'Travel', exact: true });
  expect.soft(await travel.evaluate(element => element.classList.contains('reminder-tile'))).toBe(true);
  expect.soft(await page.locator('.reminder-tiles button').last().getAttribute('aria-label')).toBe('Travel');
  expect.soft(await page.locator('.reminder-lists .reminder-list-name').allTextContents()).not.toContain('Travel');
  expect.soft(await page.locator('.reminder-pin').count()).toBe(0);
  await page.locator('.reminders-sidebar-body').evaluate(element => { element.scrollTop = element.scrollHeight; });
  if (info.project.use.viewport!.width < 700) {
    const clearance = await page.locator('.reminder-tags').evaluate(element => ({ bottom: element.getBoundingClientRect().bottom, scrollportBottom: element.closest('.reminders-sidebar-body')!.getBoundingClientRect().bottom, switcherTop: document.querySelector('.reminders-sidebar .app-menu-control')!.getBoundingClientRect().top }));
    expect.soft(clearance.bottom).toBeLessThanOrEqual(clearance.switcherTop);
    expect.soft(clearance.scrollportBottom).toBeLessThanOrEqual(clearance.switcherTop);
  }
  await page.locator('.reminders-sidebar-body').evaluate(element => { element.scrollTop = 0; });
  await openList(page, 'Personal');
  await page.getByLabel('Show Completed', { exact: true }).check();
  await expect(page.locator('.reminder-completed')).toContainText('Choose the next book');
  expect.soft(await page.locator('.reminder-completed .reminder-title').evaluate(element => getComputedStyle(element).textDecorationLine)).toBe('none');
});

test('WEB-PERF-SHELL Reminders keeps a thousand-row view virtual, responsive and available from cache', async ({ page, context, request }, info) => {
  await observePanes(page);
  await bootstrap(context, request); await control(request, '/reminders-perf');
  await page.addInitScript(() => {
    const metrics = { input: [] as number[], nextFrame: [] as number[], long: [] as number[], cls: 0, paint: 0, measuring: true, longSupported: PerformanceObserver.supportedEntryTypes.includes('longtask'), clsSupported: PerformanceObserver.supportedEntryTypes.includes('layout-shift') };
    (window as any).__remPerf = metrics;
    document.addEventListener('keydown', event => {
      if (!metrics.measuring || !event.isTrusted) return;
      const start = performance.now(); queueMicrotask(() => { document.body.getBoundingClientRect(); metrics.input.push(performance.now() - start); }); requestAnimationFrame(() => metrics.nextFrame.push(performance.now() - start));
    }, { capture: true });
    if (metrics.longSupported) new PerformanceObserver(list => { if (metrics.measuring) metrics.long.push(...list.getEntries().map(entry => entry.duration)); }).observe({ type: 'longtask' });
    if (metrics.clsSupported) new PerformanceObserver(list => { if (metrics.measuring) for (const entry of list.getEntries()) if (!(entry as any).hadRecentInput) metrics.cls += (entry as any).value; }).observe({ type: 'layout-shift' });
    new MutationObserver(() => { if (!metrics.paint && document.querySelector('.reminder-title')) { metrics.paint = -1; const painted = () => { const title = document.querySelector('.reminder-title'), rect = title?.getBoundingClientRect(); if (title && rect && rect.width > 0 && rect.left >= 0 && rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight && getComputedStyle(title).visibility === 'visible') requestAnimationFrame(() => requestAnimationFrame(() => { metrics.paint = performance.now(); })); else requestAnimationFrame(painted); }; painted(); } }).observe(document, { subtree: true, childList: true });
  });
  await page.goto('/reminders'); await openList(page, 'Bridge Test 1000 reminders');
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const navigation = await page.evaluate(() => { const value = (window as any).__remPerf; value.measuring = false; const result = { long: [...value.long], cls: value.cls, longSupported: value.longSupported, clsSupported: value.clsSupported }; value.long = []; value.cls = 0; return result; });
  if (navigation.longSupported) expect(navigation.long.filter((ms: number) => ms > 50)).toEqual([]);
  if (navigation.clsSupported) expect(navigation.cls).toBe(0);
  const list = page.getByRole('listbox', { name: 'Reminders', exact: true });
  await expect(list.getByRole('option').first()).toHaveAttribute('aria-setsize', '1000');
  expect(await list.getByRole('option').count()).toBeLessThan(80);
  await list.focus(); await page.evaluate(() => { (window as any).__remPerf.measuring = true; });
  for (let index = 0; index < 30; index++) await list.press(index % 2 ? 'PageUp' : 'PageDown');
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const measured = await page.evaluate(() => { const value = (window as any).__remPerf; value.measuring = false; return value; });
  const samples = measured.input.sort((a: number, b: number) => a - b), p95 = samples[Math.ceil(samples.length * .95) - 1];
  expect(samples).toHaveLength(30); expect(p95).toBeLessThanOrEqual(8);
  if (measured.longSupported) expect(measured.long.filter((ms: number) => ms > 50)).toEqual([]);
  if (measured.clsSupported) expect(measured.cls).toBe(0);
  await page.routeWebSocket('**/web/v1/live', () => {}); await page.reload();
  await expect(page.getByRole('listbox', { name: 'Reminders', exact: true }).getByRole('option').first()).toHaveAttribute('aria-setsize', '1000');
  await expect.poll(() => page.evaluate(() => (window as any).__remPerf.paint)).toBeGreaterThan(0);
  const warm = await page.evaluate(() => (window as any).__remPerf.paint as number); expect(warm).toBeLessThan(1000);
  const frames = measured.nextFrame.sort((a: number, b: number) => a - b); expect(frames).toHaveLength(30);
  await mkdir('../lab/evidence/web/metrics', { recursive: true });
  await writeFile(`../lab/evidence/web/metrics/WEB-PERF-REMINDERS-${info.project.name}.json`, JSON.stringify({ samples: samples.length, input_handler_p50_ms: samples[14], input_handler_p95_ms: p95, input_handler_max_ms: samples.at(-1), input_to_next_frame_p50_ms: frames[14], input_to_next_frame_p95_ms: frames[28], input_to_next_frame_max_ms: frames.at(-1), paint_timing_verified: false, warm_cached_paint_ms: warm, cls: measured.clsSupported ? measured.cls : null, long_tasks: measured.longSupported ? measured.long : null, navigation_long_tasks: navigation.longSupported ? navigation.long : null, navigation_cls: navigation.clsSupported ? navigation.cls : null, virtual_total: 1000, max_rendered_rows: 80 }) + '\n');
});
