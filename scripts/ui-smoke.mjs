// Node 22+; CHROME_BIN=/path/to/chromium node scripts/ui-smoke.mjs
// Run the Rust server first. All browser state is kept in a disposable profile.
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
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
  await send('Page.navigate',{url:process.env.APP_URL||'http://127.0.0.1:8080'});
  await evaluate(`new Promise((resolve,reject)=>{let n=0;const tick=()=>{if(document.querySelector('#status')?.textContent.startsWith('Открыт')&&!busy)resolve();else if(n++>200)reject(Error('load timeout'));else setTimeout(tick,50)};tick()})`);
  assert.equal(await evaluate('!!window.L'),true,'bundled Leaflet must load without a CDN');
  const settled = () => evaluate(`new Promise((resolve,reject)=>{let n=0;const tick=()=>{const svg=document.querySelector('.leaflet-overlay-pane svg');if(!map._animatingZoom&&!map._panAnim?._inProgress&&svg&&Math.abs(svg.getBoundingClientRect().width-Number(svg.getAttribute('width')))<.01)requestAnimationFrame(resolve);else if(n++>200)reject(Error('map did not settle'));else setTimeout(tick,25)};requestAnimationFrame(()=>requestAnimationFrame(tick))})`);
  const checkMap = async () => {
    await settled();
    const state=await evaluate(`(()=>{const svg=document.querySelector('.leaflet-overlay-pane svg');return {
      expected:scenario.routing?selectedPlan().routes.flatMap(r=>r.stops).filter(s=>s.shape).length:selectedPlan().routes.filter(r=>r.stops.length).length,
      actual:document.querySelectorAll('#map .route-line').length,
      // Regression: #map svg {height:100%;width:100%} collapsed this to 0x0.
      svg:{width:svg.getBoundingClientRect().width,height:svg.getBoundingClientRect().height,expectedWidth:Number(svg.getAttribute('width')),expectedHeight:Number(svg.getAttribute('height'))},
      paths:[...document.querySelectorAll('#map .route-line')].map(p=>({length:p.getTotalLength(),width:p.getBoundingClientRect().width,height:p.getBoundingClientRect().height})),
      flagWidth:document.querySelector('.leaflet-attribution-flag').getBoundingClientRect().width
    }})()`);
    assert.ok(state.expected>0);
    assert.equal(state.actual,state.expected,'one polyline per demo route or real street leg');
    assert.equal(state.svg.width,state.svg.expectedWidth);
    assert.equal(state.svg.height,state.svg.expectedHeight);
    assert.ok(state.flagWidth<20,'map CSS must not stretch attribution SVG');
    assert.ok(state.paths.every(p=>p.length>0&&(p.width>0||p.height>0)));
  };
  assert.equal(await evaluate('norms.length'),4,'norms.xlsx must load');
  await evaluate(`$('settings').open=true;const normInput=document.querySelectorAll('#norms input')[norms.findIndex(n=>n.key==='local')];normInput.value=35;normInput.dispatchEvent(new Event('input',{bubbles:true}));$('apply-norms').click()`);
  assert.equal(await evaluate(`scenario.jobs.filter(j=>j.work_type==='local').every(j=>j.duration===35)`),true);
  await evaluate('action(()=>solve())'); await checkMap();
  assert.equal(await evaluate(`$('settings-fields').disabled`),false,'a first plan is not an event');
  await evaluate(`(()=>{window.frozenBefore=result.plan.routes.flatMap(r=>r.stops).filter(s=>s.departure<720);$('event-time').value='12:00';$('event-time').oninput();$('urgent-type').value='local';prefillUrgent();for(const[id,value]of Object.entries({'urgent-id':'UI-URGENT','urgent-address':'UI urgent test','urgent-lat':scenario.engineers[0].start.lat,'urgent-lon':scenario.engineers[0].start.lon,'urgent-start':'12:00','urgent-end':'18:00'}))$(id).value=value;$('urgent-id').dispatchEvent(new Event('input',{bubbles:true}));$('urgent-form').requestSubmit()})()`);
  await evaluate(`new Promise((resolve,reject)=>{let n=0;const tick=()=>{if(!busy&&scenario.jobs.some(j=>j.id==='UI-URGENT'))resolve();else if(!busy&&$('status').classList.contains('error'))reject(Error($('status').textContent));else if(n++>200)reject(Error('urgent timeout'));else setTimeout(tick,50)};tick()})`);
  assert.equal(await evaluate(`scenario.jobs.find(j=>j.id==='UI-URGENT').urgent`),true);
  assert.equal(await evaluate(`frozenBefore.every(old=>JSON.stringify(result.plan.routes.flatMap(r=>r.stops).find(s=>s.job_id===old.job_id))===JSON.stringify(old))`),true);
  assert.equal(await evaluate(`$('settings-fields').disabled&&$('solve').disabled`),true,'events lock input history');
  await evaluate(`$('show-baseline').checked=true;$('show-baseline').onchange()`); await checkMap();
  await evaluate(`(async()=>{accept(await request('/api/dataset/2'));await action(()=>solve())})()`); await checkMap();
  assert.equal(await evaluate('result.stats.scope'),'candidate_routes');
  assert.equal(await evaluate('result.plan.routes.flatMap(r=>r.stops).length+result.plan.metrics.unassigned===scenario.jobs.length'),true);
  await evaluate(`action(()=>solve({kind:'cancel',time:720,job_id:$('cancel-job').value}))`); await checkMap();
  assert.equal(await evaluate(`$('solve').disabled`),true,'cannot rewrite history after an event');
  if(process.env.CITY_RESULT){
    const city=JSON.parse(await readFile(process.env.CITY_RESULT,'utf8'));
    await evaluate(`accept(${JSON.stringify(city.scenario)});result=${JSON.stringify(city)};render();lock(false)`);
    await checkMap();
    assert.equal(await evaluate('mapGeometry(selectedPlan()).lines.every(l=>l.points.length>2)'),true,'street geometry must contain road bends, not just endpoint connectors');
  }
  // Mobile layout must leave the independently-sized Leaflet overlay intact.
  await send('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
  await evaluate('renderMap()'); await checkMap();
  await evaluate('map.remove();map=null;window.L=undefined;renderMap()');
  const offline=await evaluate(`({width:document.querySelector('#map > svg').getBoundingClientRect().width,height:document.querySelector('#map > svg').getBoundingClientRect().height,routes:document.querySelectorAll('#map polyline').length,expected:scenario.routing?selectedPlan().routes.flatMap(r=>r.stops).filter(s=>s.shape).length:selectedPlan().routes.filter(r=>r.stops.length).length})`);
  assert.ok(offline.width>0&&offline.height>0); assert.equal(offline.routes,offline.expected);
  if(process.env.CITY_RESULT){
    await evaluate(`$('settings').open=true;const input=document.querySelector('#job-settings input[type=number]');input.value=Number(input.value)+.001;input.dispatchEvent(new Event('input',{bubbles:true}));$('solve').click()`);
    assert.equal(await evaluate(`!result&&!scenario.routing&&$('status').textContent.includes('Городской снимок сброшен')`),true,'stale city data must not silently fall back to straight lines');
  }
  console.log('PASS: norms, urgent form/history locks, Leaflet routes, baseline, dataset, cancellation, mobile, offline SVG'+(process.env.CITY_RESULT?', real street geometry and stale-snapshot guard':''));
} finally {
  socket?.close(); browser.kill('SIGTERM');
  await new Promise(resolve=>browser.exitCode!==null?resolve():browser.once('exit',resolve));
  await rm(profile,{recursive:true,force:true});
}
