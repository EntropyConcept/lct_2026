// Node 22+; CHROME_BIN=/path/to/chromium node scripts/ui-smoke.mjs
// UI_MOCK_ROUTING=1 isolates browser workflows from slow external routers.
// Run the Rust server first. All browser state is kept in a disposable profile.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {setTimeout as delay} from 'node:timers/promises';
const executable = process.env.CHROME_BIN;
if (!executable) throw Error('Set CHROME_BIN to a Chrome/Chromium/Helium executable');
const profile = await mkdtemp(join(tmpdir(), 'dispatch-ui-'));
const browser = spawn(executable, ['--headless=new','--no-first-run','--disable-extensions',`--user-data-dir=${profile}`,'--remote-debugging-port=0','--window-size=1400,1200','about:blank'], {stdio:'ignore'});
let socket;
try {
  let port;
  for (let i=0; i<100; i++) {
    try { port=Number((await readFile(join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]); break; }
    catch { await delay(50); }
  }
  assert.ok(port, 'browser did not start');
  const tab=(await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t=>t.type==='page');
  socket = new WebSocket(tab.webSocketDebuggerUrl);
  await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject});
  let id=0; const pending=new Map();
  socket.onmessage=({data})=>{const m=JSON.parse(data);if(m.id){const p=pending.get(m.id);pending.delete(m.id);m.error?p.reject(m.error):p.resolve(m.result)}};
  const send=(method,params={})=>new Promise((resolve,reject)=>{const n=++id;pending.set(n,{resolve,reject});socket.send(JSON.stringify({id:n,method,params}))});
  const evaluate=async expression=>{
    const r=await send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});
    if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));
    return r.result.value;
  };
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Browser.setDownloadBehavior', { behavior: 'allow', downloadPath: profile });
  const exceptions = [];
  const onmessage = socket.onmessage;
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') exceptions.push(message.params.exceptionDetails);
    onmessage(event);
  };
  await send('Page.addScriptToEvaluateOnNewDocument', { source: `
    window.__plans=[]; window.__routing=[]; window.__planRequests=[];
    const originalFetch=window.fetch;
    window.fetch=async (...args)=>{
      if(args[0]==='/api/routing') {
        const body=JSON.parse(args[1].body); window.__routing.push(body);
        if (${process.env.UI_MOCK_ROUTING === '1'}) return new Response(JSON.stringify({...body.scenario, routing:undefined}), {headers:{'Content-Type':'application/json'}});
      }
      if(args[0]==='/api/plan') window.__planRequests.push(JSON.parse(args[1].body));
      const response=await originalFetch(...args);
      if(args[0]==='/api/plan'&&response.ok)window.__plans.push(await response.clone().json());return response;
    };
  ` });
  await send('Page.navigate',{url:process.env.APP_URL||'http://127.0.0.1:8080'});
  const waitFor = async (expression, timeout = 30000) => {
    try { return await evaluate(`new Promise((resolve,reject)=>{let n=0;const tick=()=>{if(${expression})resolve();else if(n++>${timeout / 50})reject(Error('Timeout: '+${JSON.stringify(expression)}));else setTimeout(tick,50)};tick()})`); }
    catch (error) {
      console.error('Browser exceptions:', JSON.stringify(exceptions));
      console.error(await evaluate(`document.querySelector('[data-testid=map]')?.outerHTML.slice(0,1800)`));
      await screenshot('failure');
      throw error;
    }
  };
  const screenshot = async suffix => { if (process.env.SCREENSHOT_PATH) { const shot = await send('Page.captureScreenshot', { format: 'png' }); await writeFile(process.env.SCREENSHOT_PATH.replace('.png', `-${suffix}.png`), Buffer.from(shot.data, 'base64')); } };
  const clickText = text => evaluate(`(()=>{const button=[...document.querySelectorAll('button')].find(button=>button.textContent.includes(${JSON.stringify(text)}));if(!button)throw Error('Missing button: '+${JSON.stringify(text)});button.click()})()`);
  const dataset = async value => {
    const routingCount = await evaluate(`window.__routing.length`);
    await evaluate(`(()=>{const select=document.querySelector('[data-testid=dataset]');select.value=${JSON.stringify(value)};select.dispatchEvent(new Event('change',{bubbles:true}))})()`);
    await waitFor(`window.__routing.length > ${routingCount} && !document.querySelector('[data-testid=dataset]').disabled`, 1_200_000);
    assert.equal(await evaluate(`document.querySelector('[role=alert]')?.textContent || ''`), '');
  };
  await waitFor(`document.querySelector('[data-testid=dataset]')?.options.length > 3 && !document.querySelector('[data-testid=dataset]').disabled`);
  assert.equal(await evaluate(`document.body.textContent.includes('На этот день данных пока нет')`), true);
  assert.equal(await evaluate(`document.querySelector('[data-testid=solve]').disabled`), true);
  assert.equal(await evaluate(`document.querySelector('[data-testid=date-picker]').textContent.includes('сегодня')`), true);
  assert.equal(await evaluate(`document.body.textContent.includes('Параметры расчёта и дороги')`), false);
  await evaluate(`document.querySelector('[data-testid=settings-open]').click()`);
  await waitFor(`document.querySelector('[data-testid=settings-dialog]')?.open`);
  assert.equal(await evaluate(`document.querySelector('[data-testid=solver-mode]').value`), 'exact');
  assert.equal(await evaluate(`document.querySelector('[data-testid=solver-seconds]').value`), '30');
  // Keep this workflow smoke test to one attempt; model tests cover refinement.
  await evaluate(`(()=>{const input=document.querySelector('[data-testid=solver-seconds]');input.value='1';input.dispatchEvent(new Event('input',{bubbles:true}))})()`);
  await screenshot('settings');
  await send('Input.dispatchKeyEvent', { type:'keyDown', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 });
  await send('Input.dispatchKeyEvent', { type:'keyUp', key:'Escape', code:'Escape', windowsVirtualKeyCode:27 });
  await waitFor(`!document.querySelector('[data-testid=settings-dialog]')`);
  assert.equal(await evaluate(`document.activeElement.dataset.testid`), 'settings-open');
  await dataset('/api/demo');
  assert.equal(await evaluate(`window.__routing.length`), 1);
  assert.equal(await evaluate(`window.__routing[0].scenario.transit_date`), await evaluate(`new Date().toLocaleDateString('sv-SE')`));
  await evaluate(`document.querySelector('[data-testid=settings-open]').click()`);
  await waitFor(`document.querySelector('[data-testid=settings-dialog]')?.open`);
  await screenshot('settings-loaded');
  await evaluate(`document.querySelector('[aria-label="Закрыть настройки"]').click()`);
  await clickText('инженеров');
  assert.equal(await evaluate(`document.querySelectorAll('[data-testid=engineers] tbody tr').length`),5);
  await evaluate(`document.querySelector('[data-testid=solve]').click()`);
  await waitFor(`window.__plans.length === 1 && !document.querySelector('[data-testid=solve]').disabled`);
  assert.deepEqual(await evaluate(`({mode:window.__planRequests[0].mode, seconds:window.__planRequests[0].seconds})`), {mode:'exact', seconds:1});
  assert.ok(await evaluate(`Number(document.querySelector('[data-testid=map]').dataset.routeCount)`) > 0);
  await waitFor(`document.querySelectorAll('[data-testid=map] .dispatch-marker').length > 0 || document.querySelectorAll('[data-testid=map] svg polyline').length > 0`);
  await waitFor(`Number(document.querySelector('[data-testid=map]').dataset.renderedRoutes) > 0 || document.querySelectorAll('[data-testid=map] svg polyline').length > 0`);
  const mapSize = await evaluate(`(()=>{const surface=document.querySelector('[data-testid=map] .maplibregl-canvas') || document.querySelector('[data-testid=map] svg');const rect=surface.getBoundingClientRect();return {width:rect.width,height:rect.height}})()`);
  assert.ok(mapSize.width > 100 && mapSize.height > 100, 'Map drawing surface must have visible dimensions');
  const mapWidth = await evaluate(`document.querySelector('[data-testid=map]').getBoundingClientRect().width`);
  await evaluate(`document.querySelector('.dispatch-marker[title^="J02:"]').click()`);
  await waitFor(`document.querySelector('[data-testid=map-details]')`);
  assert.equal(await evaluate(`!!document.querySelector('[data-testid=sidebar-editor]')`), false, 'map click must not open the sidebar');
  assert.equal(await evaluate(`document.querySelector('[data-testid=map-details]').textContent.includes('Исполнитель')`), true);
  assert.equal(await evaluate(`document.querySelector('[data-testid=map]').getBoundingClientRect().width`), mapWidth);
  await screenshot('job-popup');
  await clickText('Редактировать');
  await waitFor(`document.querySelector('[data-testid=map-details] [data-testid=editor-save]')`);
  await evaluate(`window.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape'}))`);
  await evaluate(`document.querySelector('.engineer-marker').click()`);
  await waitFor(`document.querySelector('[data-testid=map-details]')?.textContent.includes('Расписание')`);
  assert.equal(await evaluate(`!!document.querySelector('[data-testid=sidebar-editor]')`), false);
  await screenshot('engineer-popup');
  await evaluate(`document.querySelector('[aria-label="Закрыть детали"]').click()`);
  await evaluate(`document.querySelector('[data-testid=engineers] tbody tr').click()`);
  await waitFor(`document.querySelector('[data-testid=engineer-details]')`);
  assert.equal(await evaluate(`document.querySelector('[data-testid=engineer-details]').textContent.includes('Маршрут и расписание')`), true);
  await screenshot('details');
  await evaluate(`document.querySelector('[data-testid=live-mode]').click()`);
  await waitFor(`document.querySelector('[data-testid=timeline]')`);
  await screenshot('timeline');
  const travel = await evaluate(`(()=>{for(const route of window.__plans[0].plan.routes){const stop=route.stops.find(stop=>stop.arrival-stop.departure>0);if(stop)return {engineer:route.engineer_id,departure:stop.departure};}throw Error('No travel fixture')})()`);
  await evaluate(`(()=>{const input=document.querySelector('[aria-label="Время событий"]');input.value=${travel.departure};input.dispatchEvent(new Event('input',{bubbles:true}));window.__engineerMarker=[...document.querySelectorAll('.engineer-marker')].find(marker=>marker.dataset.engineerId===${JSON.stringify(travel.engineer)})})()`);
  await evaluate(`(()=>{const speed=document.querySelector('[aria-label="Скорость воспроизведения"]');speed.value='1';speed.dispatchEvent(new Event('change',{bubbles:true}))})()`);
  await evaluate(`document.querySelector('[data-testid=live-play]').click()`);
  await waitFor(`document.querySelector('[aria-label="Приостановить"]')`);
  await delay(150);
  const movingPosition = await evaluate(`window.__engineerMarker.style.transform`);
  await delay(250);
  assert.notEqual(await evaluate(`window.__engineerMarker.style.transform`), movingPosition, 'engineer moves smoothly between clock ticks');
  assert.equal(await evaluate(`window.__engineerMarker.isConnected`), true, 'movement retains the same marker');
  assert.equal(await evaluate(`!!window.__engineerMarker.querySelector('svg')`), true, 'transport icon is rendered');
  assert.equal(await evaluate(`window.__engineerMarker.querySelector('.engineer-progress').hidden`), false);
  await screenshot('moving-engineer');
  await evaluate(`document.querySelector('[aria-label="Приостановить"]').click()`);
  await delay(100);
  const pausedPosition = await evaluate(`window.__engineerMarker.style.transform`);
  await delay(400);
  assert.equal(await evaluate(`window.__engineerMarker.style.transform`), pausedPosition, 'paused marker stays still');
  const beforeZoom = await evaluate(`document.querySelector('.live-schedule').getBoundingClientRect().width`);
  await evaluate(`document.querySelector('[aria-label="Увеличить масштаб расписания"]').click()`);
  await waitFor(`document.querySelector('.live-schedule').getBoundingClientRect().width > ${beforeZoom}`);
  await evaluate(`document.querySelector('.event-work').click()`);
  await waitFor(`document.querySelector('[data-testid=map-details]')`);
  assert.equal(await evaluate(`!!document.querySelector('[data-testid=timeline]')`), true, 'event selection keeps Live open');
  assert.equal(await evaluate(`!!document.querySelector('[data-testid=sidebar-editor]')`), false, 'timeline selection uses map popup');
  await evaluate(`document.querySelector('[aria-label="Закрыть детали"]').click()`);
  // Restore event time used by history assertions below.
  await evaluate(`(()=>{const input=document.querySelector('[aria-label="Время событий"]');input.value=720;input.dispatchEvent(new Event('input',{bubbles:true}))})()`);
  await clickText('заявок');
  await clickText('Срочная заявка');
  await evaluate(`(()=>{const input=document.querySelector('[data-testid=job-address]');input.value='UI urgent test';input.dispatchEvent(new Event('input',{bubbles:true}))})()`);
  await waitFor(`!document.querySelector('[data-testid=editor-save]').disabled`);
  await evaluate(`document.querySelector('[data-testid=editor-save]').click()`);
  await waitFor(`window.__plans.length === 2 && !document.querySelector('[data-testid=dataset]').disabled`);
  const plans = await evaluate('window.__plans');
  assert.equal(plans[1].scenario.jobs.length, plans[0].scenario.jobs.length + 1);
  assert.equal(plans[1].scenario.jobs.at(-1).urgent, true);
  for (const stop of plans[0].plan.routes.flatMap(route => route.stops).filter(stop => stop.departure < 720)) {
    assert.deepEqual(plans[1].plan.routes.flatMap(route => route.stops).find(next => next.job_id === stop.job_id), stop);
  }
  assert.equal(await evaluate(`document.querySelector('[data-testid=solve]').disabled`), true, 'events lock history');
  // Import two dated buckets through the actual file input.
  await evaluate(`(()=>{const row=[...document.querySelectorAll('tbody tr')].find(row=>row.textContent.includes(window.__plans[1].scenario.jobs.at(-1).id));if(!row)throw Error('New urgent row not found');row.click()})()`);
  await clickText('Отменить заявку');
  await waitFor(`window.__plans.length === 3 && !document.querySelector('[data-testid=dataset]').disabled`);
  assert.equal(await evaluate(`window.__plans[2].scenario.jobs.length`), plans[0].scenario.jobs.length);
  const fixture = join(profile, 'days.json');
  await writeFile(fixture, JSON.stringify({ days: [
    { date: '2026-09-29', scenario: { ...plans[0].scenario, name: 'First day' } },
    { date: '2026-09-30', scenario: { ...plans[0].scenario, name: 'Second day', jobs: plans[0].scenario.jobs.slice(0, 2) } },
  ] }));
  const upload = async path => {
    const document = await send('DOM.getDocument');
    const { nodeId } = await send('DOM.querySelector', { nodeId: document.root.nodeId, selector: '[data-testid=import]' });
    await send('DOM.setFileInputFiles', { nodeId, files: [path] });
    await waitFor(`document.querySelector('[data-testid=import]').value === '' && !document.querySelector('[data-testid=dataset]').disabled`, 1_200_000);
  };
  await upload(fixture);
  assert.equal(await evaluate(`document.querySelector('[role=alert]')?.textContent || ''`), '');
  assert.deepEqual(await evaluate(`window.__routing.slice(-2).map(call=>call.scenario.transit_date)`), ['2026-09-29','2026-09-30']);
  await evaluate(`document.querySelector('[data-testid=date-picker]').click()`);
  await waitFor(`document.querySelector('button[data-day="2026-09-30"]')`);
  await evaluate(`document.querySelector('button[data-day="2026-09-30"]').click()`);
  await waitFor(`document.body.textContent.includes('Заявки · 2')`);
  await clickText('Экспорт данных');
  let exported;
  for (let i=0; i<100; i++) {
    try { exported = JSON.parse(await readFile(join(profile, 'dispatch-days.json'), 'utf8')); break; } catch { await delay(50); }
  }
  assert.equal(exported.days.find(day => day.date === '2026-09-30').scenario.jobs.length, 2);
  assert.equal(exported.days.find(day => day.date === '2026-09-29').scenario.jobs.length, plans[0].scenario.jobs.length);
  // Failed imports are transactional, retaining the selected day's data.
  await writeFile(fixture, JSON.stringify({ days: [{ date: '2026-09-30', scenario: plans[0].scenario }, { date: 'bad-date', scenario: {} }] }));
  await upload(fixture);
  await waitFor(`document.querySelector('[role=alert]')`);
  assert.equal(await evaluate(`document.body.textContent.includes('Заявки · 2')`), true);
  await dataset('/api/dataset/2');
  assert.equal(await evaluate(`document.querySelector('[role=alert]')?.textContent || ''`), '');
  await evaluate(`document.querySelector('[data-testid=solve]').click()`);
  await waitFor(`window.__plans.length === 4 && !document.querySelector('[data-testid=dataset]').disabled`);
  await evaluate(`document.querySelector('[data-testid=live-mode]').click()`);
  await waitFor(`document.querySelector('[data-testid=timeline]')`);
  await screenshot('dense-timeline');
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('.live-event')).filter(event=>event.clientWidth<46).every(event=>getComputedStyle(event.querySelector('.event-title')).display==='none')`), true, 'short events hide overflowing labels');
  await evaluate(`document.querySelector('.live-scroll').scrollLeft=250`);
  assert.ok(await evaluate(`Math.abs(document.querySelector('.live-person').getBoundingClientRect().left-document.querySelector('.live-scroll').getBoundingClientRect().left)<2`), 'engineer column stays pinned');
  await send('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  assert.ok(await evaluate(`document.querySelector('[data-testid=map]').getBoundingClientRect().height`) > 0);
  assert.deepEqual(exceptions, [], 'No uncaught browser errors');
  if (process.env.SCREENSHOT_PATH) {
    await send('Emulation.clearDeviceMetricsOverride');
    const screenshot = await send('Page.captureScreenshot', { format: 'png' });
    await writeFile(process.env.SCREENSHOT_PATH, Buffer.from(screenshot.data, 'base64'));
  }
  console.log('PASS: settings modal/Escape/focus, today, exact SAT 5s, automatic road preparation, empty/loaded states, engineers, solve, map popups/edit/Escape, route details, live engineer motion/pause/progress/zoom/event details/dense schedule, urgent/cancel history, calendar, multi-day/transactional import, export, CSV, mobile');

} finally {
  socket?.close(); browser.kill('SIGTERM');
  await new Promise(resolve=>browser.exitCode!==null?resolve():browser.once('exit',resolve));
  await rm(profile,{recursive:true,force:true});
}
