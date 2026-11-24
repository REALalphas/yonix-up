import {test,after} from 'node:test';
import assert from 'node:assert/strict';
import {createServer} from 'vite';
import {renderToStaticMarkup} from 'react-dom/server';
import {createElement} from 'react';
import {readFileSync,readdirSync} from 'node:fs';
const server=await createServer({configFile:'frontend/vite.config.ts',server:{middlewareMode:true},appType:'custom'});
after(()=>server.close());
const model=await server.ssrLoadModule('/model.ts');
const i18n=await server.ssrLoadModule('/i18n.ts');
test('all 13 shared catalogs cover every English key',()=>{assert.equal(Object.keys(i18n.catalogs).length,13);for(const [language,strings] of Object.entries(i18n.catalogs)){assert.deepEqual(Object.keys(strings).sort(),Object.keys(i18n.catalogs.en).sort(),language);for(const value of Object.values(strings))assert.ok(value.length);}});
test('language resolves regional locales and fallback',()=>{assert.equal(i18n.resolveLanguage('system',['ru-RU']),'ru');assert.equal(i18n.resolveLanguage('de',['ru-RU']),'de');assert.equal(i18n.resolveLanguage('system',['xx','ja-JP']),'ja');assert.equal(i18n.resolveLanguage('bogus',['xx']),'en');});
test('policy blocks concurrent and privileged operations',()=>{const status={active:null,system_changes_enabled:false};for(const action of ['switch','rollback','clean'])assert.equal(model.allowed(action,status,false),false);for(const action of ['check','build','diff','update-inputs'])assert.equal(model.allowed(action,status,false),true);assert.equal(model.allowed('build',{...status,active:'job'},false),false);assert.equal(model.allowed('check',status,true),false);assert.equal(model.allowed('check',null,false),false);assert.equal(model.allowed('switch',{...status,system_changes_enabled:true},false),true);});
test('legacy environments cannot update flake inputs',()=>{const base={active:null,system_changes_enabled:true};assert.equal(model.allowed('update-inputs',{...base,config_kind:'flake'},false),true);assert.equal(model.allowed('update-inputs',{...base},false),true);assert.equal(model.allowed('update-inputs',{...base,config_kind:'legacy'},false),false);assert.equal(model.allowed('update-inputs',{...base,config_kind:'unknown'},false),false);assert.equal(model.allowed('switch',{...base,config_kind:'legacy'},false),true);assert.equal(model.allowed('build',{...base,config_kind:'legacy'},false),true);});
test('inaccessible or missing configurations block only checkout operations', () => {
    const status = { active: null, system_changes_enabled: true, config_kind: 'unknown' };
    for (const action of ['check', 'build', 'diff', 'switch', 'update-inputs']) {
        assert.equal(model.allowed(action, status, false), false);
    }
    for (const action of ['rollback', 'clean']) {
        assert.equal(model.allowed(action, status, false), true);
    }
});
test('all mutating actions have explicit confirmation copy',()=>{for(const action of ['switch','rollback','clean','update-inputs'])assert.ok(i18n.catalogs.en[model.confirmations[action]]);assert.equal(model.confirmations.check,undefined);});
test('API authenticates and sends only explicit operation data',async()=>{const original=globalThis.fetch;let request;globalThis.fetch=async(path,options)=>{request={path,...options};return new Response('{"id":"job"}',{headers:{'Content-Type':'application/json'}});};try{await model.api('private','jobs',undefined,{action:'check',confirmed:false});assert.equal(request.path,'/api/jobs');assert.equal(request.headers.Authorization,'Bearer private');assert.deepEqual(JSON.parse(request.body),{action:'check',confirmed:false});assert.equal(request.method,'POST');}finally{globalThis.fetch=original;}});
test('API errors are readable and logs remain plain text',async()=>{const original=globalThis.fetch;try{globalThis.fetch=async()=>new Response('{"error":"Busy"}',{status:409});await assert.rejects(model.api('x','jobs'),/Busy/);globalThis.fetch=async()=>new Response('<script>untrusted log</script>');assert.equal(await model.api('x','jobs/id/log'),'<script>untrusted log</script>');}finally{globalThis.fetch=original;}});
test('API distinguishes rejected credentials from network failures',async()=>{const original=globalThis.fetch;try{globalThis.fetch=async()=>new Response('{"error":"Access token required"}',{status:401});await assert.rejects(model.api('invalid','hosts/local/status'),error=>error instanceof model.ApiError&&error.status===401&&error.message==='Access token required');globalThis.fetch=async()=>{throw new TypeError('Failed to fetch');};await assert.rejects(model.api('x','status'),error=>error instanceof TypeError&&!(error instanceof model.ApiError));}finally{globalThis.fetch=original;}});
test('real Alpha UI login renders in every language with no saved token',async()=>{const {App}=await server.ssrLoadModule('/App.tsx');for(const language of Object.keys(i18n.catalogs)){Object.defineProperty(globalThis,'navigator',{value:{languages:[language]},configurable:true});globalThis.localStorage={getItem:key=>key==='yonix-language'?language:null};const html=renderToStaticMarkup(createElement(App));assert.ok(html.includes('as-card'));assert.ok(html.includes('as-field'));assert.ok(html.includes('as-button'));assert.ok(html.includes(i18n.catalogs[language].connect));assert.ok(html.includes('type="password"'));}delete globalThis.localStorage;});
test('production assets are bundled locally and old client is gone',()=>{const html=readFileSync('web/index.html','utf8');assert.match(html,/type="module"/);assert.doesNotMatch(html,/https?:\/\//);const assets=readdirSync('web/assets');assert.ok(assets.some(a=>a.endsWith('.woff2')));assert.ok(!readdirSync('web').includes('app.js'));});
test('host API paths bind jobs and logs to a single selected host',()=>{assert.equal(model.hostPath('web','jobs/job/log'),'hosts/web/jobs/job/log');assert.equal(model.hostPath('local','status'),'hosts/local/status');assert.equal(model.hostPath('odd/id','status'),'hosts/odd%2Fid/status');});
test('saved credentials restore only valid tokens and can be cleared without affecting language',()=>{const values=new Map([['yonix-language','ru']]);globalThis.localStorage={getItem:key=>values.get(key)??null,setItem:(key,value)=>values.set(key,value),removeItem:key=>values.delete(key)};try{assert.equal(model.storedToken(),'');assert.equal(model.saveToken('a'.repeat(64)),true);assert.equal(model.storedToken(),'a'.repeat(64));model.clearToken();assert.equal(model.storedToken(),'');assert.equal(values.get('yonix-language'),'ru');values.set('yonix-access-token','broken');assert.equal(model.storedToken(),'');}finally{delete globalThis.localStorage;}});
test('blocked browser storage still permits a memory-only session',()=>{globalThis.localStorage={getItem:()=>{throw new Error('blocked');},setItem:()=>{throw new Error('blocked');},removeItem:()=>{throw new Error('blocked');}};try{assert.equal(model.storedToken(),'');assert.equal(model.saveToken('a'.repeat(64)),false);assert.doesNotThrow(()=>model.clearToken());}finally{delete globalThis.localStorage;}});
test('history orders jobs by start time, preserves ties, and shows the selected failure', async () => {
    const {HistoryPage} = await server.ssrLoadModule('/History.tsx');
    // The daemon returns newest first. Two jobs started during the same second.
    const jobs = [
        {id: 'new-job', action: 'build', status: 'failed', started: 20, finished: 21, error: 'Build failed'},
        {id: 'selected-job', action: 'check', status: 'failed', started: 20, finished: 20, error: 'Check failed'},
        {id: 'first-job', action: 'diff', status: 'succeeded', started: 10, finished: 11},
    ];
    const html = renderToStaticMarkup(createElement(HistoryPage, {
        jobs, selected: 'selected-job', log: 'selected job log', host: 'nas', language: 'en',
        t: key => i18n.catalogs.en[key], onSelect: () => {}, onSystem: () => {},
    }));

    const detailsStart = html.indexOf('<h2');
    assert.ok(detailsStart >= 0, 'selected operation heading is missing');
    const strip = html.slice(0, detailsStart);
    const details = html.slice(detailsStart);
    const labels = [...strip.matchAll(/<button\b[^>]*>([\s\S]*?)<\/button>/g)]
        .map(([, content]) => content.replace(/<[^>]+>/g, '').trim());
    const en = i18n.catalogs.en;
    assert.deepEqual(labels, [en.actionDiff, en.actionCheck, en.actionBuild]);
    assert.ok(!strip.includes(en.operationId));
    assert.ok(!strip.includes('selected job log'));
    assert.ok(details.includes('selected-job'));
    assert.ok(details.includes('Check failed'));
    assert.ok(details.includes('selected job log'));
    assert.ok(!details.includes('Build failed'));
});
test('action status keys map operations to informative statuses',()=>{assert.equal(model.actionStatusKey('check'),'statusChecking');assert.equal(model.actionStatusKey('build'),'statusBuilding');assert.equal(model.actionStatusKey('diff'),'statusBuilding');assert.equal(model.actionStatusKey('switch'),'statusUpdating');assert.equal(model.actionStatusKey('update-inputs'),'statusUpdating');assert.equal(model.actionStatusKey('rollback'),'statusRollingBack');assert.equal(model.actionStatusKey('clean'),'statusCleaning');assert.equal(model.actionStatusKey(null),'working');assert.equal(i18n.catalogs.en.statusChecking,'Checking');assert.equal(i18n.catalogs.en.statusBuilding,'Building');assert.equal(i18n.catalogs.en.statusUpdating,'Updating');assert.equal(i18n.catalogs.en.statusRollingBack,'Rolling back');assert.equal(i18n.catalogs.en.statusCleaning,'Cleaning');assert.equal(i18n.catalogs.ru.statusChecking,'Проверка');assert.equal(i18n.catalogs.ru.statusBuilding,'Сборка');assert.equal(i18n.catalogs.ru.statusUpdating,'Обновление');assert.equal(i18n.catalogs.ru.statusRollingBack,'Откат');assert.equal(i18n.catalogs.ru.statusCleaning,'Очистка');});
test('resolveHostStatus resolves action from active_action or jobs list',()=>{assert.equal(model.resolveHostStatus(null,false,''),'connectingShort');assert.equal(model.resolveHostStatus(null,true,'t'),'connectionOffline');assert.equal(model.resolveHostStatus(null,false,'t'),'connectingShort');const idleStatus={active:null,host:'nas',config_path:'/etc',current_system:null,system_changes_enabled:true};assert.equal(model.resolveHostStatus(idleStatus,false,'t'),null);const activeWithAction={...idleStatus,active:'1',active_action:'build'};assert.equal(model.resolveHostStatus(activeWithAction,false,'t'),'statusBuilding');const activeFallbackJob={...idleStatus,active:'2'};const jobs=[{id:'2',action:'rollback',status:'running',started:1}];assert.equal(model.resolveHostStatus(activeFallbackJob,false,'t',jobs),'statusRollingBack');});
