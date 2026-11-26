import assert from 'node:assert/strict';
import { after, test } from 'node:test';
import { Window } from 'happy-dom';
import { act, createElement } from 'react';
import { createServer } from 'vite';

const window = new Window({
    url: 'http://127.0.0.1:9847',
    settings: { disableJavaScriptEvaluation: true, disableJavaScriptFileLoading: true, disableCSSFileLoading: true },
});
const { document } = window;
for (const name of ['document', 'navigator', 'localStorage', 'HTMLElement']) {
    Object.defineProperty(globalThis, name, { configurable: true, value: window[name] });
}
globalThis.window = window;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

const { createRoot } = await import('react-dom/client');
const vite = await createServer({
    configFile: 'frontend/vite.config.ts',
    server: { middlewareMode: true, hmr: false, ws: false },
    appType: 'custom',
});
const { App } = await vite.ssrLoadModule('/App.tsx');
const { ConnectionsPage } = await vite.ssrLoadModule('/Connections.tsx');
const { catalogs } = await vite.ssrLoadModule('/i18n.ts');
const en = catalogs.en;
after(async () => {
    await vite.close();
    await window.happyDOM.close();
});

const token = 'a'.repeat(64);
const tokenKey = 'yonix-access-token';
const localHost = { id: 'local', label: 'Gateway fixture', local: true };
const remoteHost = { id: 'peer-1', label: 'Peer fixture', local: false };
const status = {
    version: '0.1.0',
    host: 'gateway',
    config_path: '/fixture/config',
    current_system: null,
    active: null,
    system_changes_enabled: true,
    config_kind: 'flake',
};

function mockApi(t, handle) {
    const requests = [];
    const fixtureErrors = [];
    t.after(() => { assert.deepEqual(fixtureErrors, []); });
    t.mock.method(globalThis, 'fetch', async (url, options) => {
        try {
            assert.match(url, /^\/api\//);
            const request = {
                path: url.slice('/api/'.length),
                method: options.method,
                auth: options.headers.Authorization,
                body: options.body ? JSON.parse(options.body) : undefined,
                signal: options.signal,
            };
            assert.equal(request.auth, 'Bearer ' + token);
            requests.push(request);
            return await handle(request);
        } catch (error) {
            // Components catch request failures. Surface failed fixture assertions
            // even when the test's UI assertions would otherwise pass.
            if (error instanceof assert.AssertionError) fixtureErrors.push(error);
            throw error;
        }
    });
    return requests;
}

const json = (data, status = 200) => Response.json(data, { status });

async function mount(t, component, { signedIn = false } = {}) {
    localStorage.clear();
    localStorage.setItem('yonix-language', 'en');
    if (signedIn) localStorage.setItem(tokenKey, token);
    const target = document.body.appendChild(document.createElement('main'));
    const root = createRoot(target);
    t.after(async () => {
        await act(() => root.unmount());
        target.remove();
        localStorage.clear();
    });
    await act(async () => { root.render(component); });
    return target;
}

function element(target, selector) {
    const result = target.querySelector(selector);
    assert.ok(result, 'Missing element ' + selector);
    return result;
}

function button(target, label) {
    const result = [...target.querySelectorAll('button')].find(
        node => node.textContent.trim() === label || node.getAttribute('aria-label') === label,
    );
    assert.ok(result, 'Missing button ' + label);
    return result;
}

const click = node => act(async () => { node.click(); });

async function type(input, value) {
    const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
    await act(() => {
        setter.call(input, value);
        input.dispatchEvent(new window.Event('input', { bubbles: true }));
    });
}

async function selectHost(view, id) {
    const select = element(view, 'select');
    await act(async () => {
        select.value = id;
        select.dispatchEvent(new window.Event('change', { bubbles: true }));
    });
    assert.equal(select.value, id);
}

async function press(element, key) {
    await act(() => {
        element.dispatchEvent(new window.KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
        element.dispatchEvent(new window.KeyboardEvent('keyup', { key, bubbles: true, cancelable: true }));
    });
}

test('login trims and persists a token, and a later 401 invalidates the session', async t => {
    let rejected = false;
    const jobs = [{ id: 'old-job', action: 'build', status: 'succeeded', started: 1, finished: 2 }];
    const requests = mockApi(t, ({ path }) => {
        switch (path) {
            case 'status':
            case 'hosts/local/status': return json(status);
            case 'hosts/local/jobs': return json(jobs);
            case 'hosts': return json([localHost]);
            case 'hosts/local/jobs/old-job/log': return rejected ? json({ error: 'expired' }, 401) : new Response('build log');
            default: assert.fail('Unexpected request ' + path);
        }
    });
    const view = await mount(t, createElement(App));
    assert.equal(button(view, en.connect).disabled, true);
    await type(element(view, '#token'), '  ' + token + ' ');
    assert.equal(button(view, en.connect).disabled, false);
    await click(button(view, en.connect));

    assert.equal(localStorage.getItem(tokenKey), token);
    assert.equal(button(view, en.actionBuild).disabled, false);
    assert.ok(view.textContent.includes(`${en.version}v0.1.0`));
    assert.ok(requests.some(request => request.path === 'hosts/local/status'));

    rejected = true;
    await click(button(view, en.history));
    assert.equal(localStorage.getItem(tokenKey), null);
    assert.equal(localStorage.getItem('yonix-language'), 'en');
    assert.ok(element(view, '[role="alert"]').textContent.includes(en.authenticationHelp));
    assert.equal(element(view, '#token').value, '');
});

test('a transient status error keeps the saved session and disables operations', async t => {
    mockApi(t, ({ path }) => {
        if (path === 'hosts/local/status') throw new TypeError('fixture offline');
        if (path === 'hosts/local/jobs') return json([]);
        if (path === 'hosts') return json([localHost]);
        assert.fail('Unexpected request ' + path);
    });
    const view = await mount(t, createElement(App), { signedIn: true });
    assert.match(element(view, '[role="alert"]').textContent, /fixture offline/);
    assert.equal(button(view, en.actionBuild).disabled, true);
    assert.equal(localStorage.getItem(tokenKey), token);
});

test('confirmation targets the selected host and successful submission opens its text log', async t => {
    const job = { id: 'remote-job', action: 'switch', status: 'running', started: 1 };
    let jobStarted = false;
    const requests = mockApi(t, ({ path, method }) => {
        switch (path) {
            case 'hosts': return json([localHost, remoteHost]);
            case 'hosts/local/status': return json(status);
            case 'hosts/local/jobs': return json([]);
            case 'hosts/peer-1/status': return json({ ...status, version: '0.2.0', host: 'remote', active: jobStarted ? job.id : null });
            case 'hosts/peer-1/jobs':
                if (method === 'POST') {
                    jobStarted = true;
                    return json(job);
                }
                return json(jobStarted ? [job] : []);
            case 'hosts/peer-1/jobs/remote-job/log': return new Response('<img src=x> remote log');
            default: assert.fail('Unexpected request ' + path);
        }
    });
    const view = await mount(t, createElement(App), { signedIn: true });
    await selectHost(view, remoteHost.id);
    assert.ok(view.textContent.includes('remote'));
    assert.ok(view.textContent.includes(`${en.version}v0.2.0`));

    await click(button(view, en.actionSwitch));
    assert.ok(element(view, '[role="alert"]').textContent.includes(remoteHost.label));
    assert.equal(requests.filter(request => request.method === 'POST').length, 0);
    await click(button(view, en.cancel));
    assert.equal(view.querySelector('[role="alert"]'), null);

    await click(button(view, en.actionSwitch));
    await click(button(view, en.continue));
    assert.deepEqual(requests.filter(request => request.method === 'POST').map(({ path, body }) => ({ path, body })), [
        { path: 'hosts/peer-1/jobs', body: { action: 'switch', confirmed: true } },
    ]);
    const log = element(view, '[role="region"][aria-label="' + en.operationLog + '"]');
    assert.equal(log.textContent, '<img src=x> remote log');
    assert.equal(log.querySelector('img'), null);
});

function connectionsPage(pairingSession, changes = () => {}) {
    return createElement(ConnectionsPage, {
        token,
        gateway: localHost.label,
        pairingSession,
        t: key => en[key],
        onChange: changes,
        onAuthenticationError: error => assert.fail(error),
    });
}

function pairingFields(view) {
    return [...view.querySelectorAll('input[aria-label^="' + en.pairingCode + ' "]')];
}

test('pairing share opens from the keyboard, falls back to manual copy and revokes on back', async t => {
    const link = '~fixture#AB3K9Q';
    const requests = mockApi(t, ({ path, method }) => {
        if (path === 'connections') return json([]);
        if (path === 'pairings') {
            return method === 'GET' ? json({ enabled: true }) : json({ code: 'AB3K9Q', link, expires_in: 300 });
        }
        if (path === 'pairings/revoke') return json({ revoked: true });
        assert.fail('Unexpected request ' + path);
    });
    t.mock.method(window.navigator.clipboard, 'writeText', async () => { throw new Error('clipboard denied'); });
    const session = { invite: null, receivedPairs: null };
    const view = await mount(t, connectionsPage(session));
    const add = element(view, '[role="button"][aria-label="' + en.addConnection + '"]');
    await press(add, 'Enter');

    assert.equal(pairingFields(view).map(input => input.value).join(''), 'AB3K9Q');
    assert.equal(session.invite.link, link);
    assert.equal(document.activeElement, button(view, en.back));
    await click(button(view, en.copyPairingLink));
    const manual = [...view.querySelectorAll('input[readonly]')].find(input => input.getAttribute('aria-label') === en.pairingLink);
    assert.ok(manual, 'Manual copy field should be shown after clipboard denial');
    assert.equal(manual.value, link);
    assert.equal(document.activeElement, manual);
    await click(button(view, en.back));
    assert.equal(session.invite, null);
    assert.ok(view.querySelector('[role="button"][aria-label="' + en.addConnection + '"]'));
    assert.equal(requests.filter(({ path }) => path === 'pairings/revoke').length, 1);
});

test('joining normalizes a pasted link and omits an empty origin', async t => {
    let paired = false;
    let changes = 0;
    const entry = { id: 'peer-saved', label: 'Saved peer fixture', url: 'http://127.0.0.2:9847', managed_by_nix: false };
    const requests = mockApi(t, ({ path, method }) => {
        if (path === 'connections') return json(paired ? [entry] : []);
        if (path === 'pairings') {
            return method === 'GET' ? json({ enabled: true }) : json({ code: 'AB3K9Q', link: '~own#AB3K9Q', expires_in: 300 });
        }
        if (path === 'pairings/accept') {
            paired = true;
            return json(entry);
        }
        if (path === 'connections/peer-saved/check') return json({ host: 'peer', version: 'fixture' });
        if (path === 'pairings/revoke') return json({ revoked: true });
        assert.fail('Unexpected request ' + path);
    });
    const view = await mount(t, connectionsPage({ invite: null, receivedPairs: null }, async () => { changes++; }));
    await click(element(view, '[role="button"][aria-label="' + en.addConnection + '"]'));
    await click(button(view, en.addOtherInstance));
    assert.equal(button(view, en.pairingAccept).disabled, true);

    const fields = pairingFields(view);
    assert.equal(document.activeElement, fields[0]);
    const paste = new window.Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(paste, 'clipboardData', { value: { getData: () => ' ~fixture#ab3-k9q ' } });
    await act(() => { fields[0].dispatchEvent(paste); });
    assert.equal(paste.defaultPrevented, true);
    assert.equal(document.activeElement, fields.at(-1));
    assert.equal(fields.map(input => input.value).join(''), 'AB3K9Q');

    await click(button(view, en.pairingAccept));
    assert.deepEqual(requests.find(({ path }) => path === 'pairings/accept').body, {
        link: '~fixture#AB3K9Q',
        allow_public: false,
    });
    assert.equal(changes, 1);
    assert.ok(view.textContent.includes(entry.label));
    assert.ok(view.textContent.includes(en.pairingPaired));
});

test('saved connections show failed checks and require a separate remove confirmation', async t => {
    const entries = [
        { id: 'manual', label: 'Manual fixture', url: 'http://127.0.0.2:9847', managed_by_nix: false },
        { id: 'declared', label: 'Declared fixture', url: 'http://127.0.0.3:9847', managed_by_nix: true },
    ];
    let removed = false;
    const requests = mockApi(t, ({ path }) => {
        switch (path) {
            case 'pairings': return json({ enabled: false });
            case 'connections': return json(removed ? entries.slice(1) : entries);
            case 'connections/manual/check': return json({ error: 'offline' }, 502);
            case 'connections/declared/check': return json({ host: 'declared' });
            case 'connections/manual/remove':
                removed = true;
                return json({ removed: true });
            default: assert.fail('Unexpected request ' + path);
        }
    });
    const view = await mount(t, connectionsPage({ invite: null, receivedPairs: null }));
    assert.ok(view.textContent.includes(en.connectionManagedByNix));
    assert.ok(view.textContent.includes(en.connectionOffline));
    assert.equal([...view.querySelectorAll('button')].filter(node => node.textContent.trim() === en.removeConnection).length, 1);

    await click(button(view, en.removeConnection));
    assert.ok(view.textContent.includes(en.removeConnectionHelp));
    assert.ok(requests.every(({ path }) => !path.endsWith('/remove')));
    await click(button(view, en.cancel));
    assert.ok(!view.textContent.includes(en.removeConnectionHelp));

    await click(button(view, en.removeConnection));
    await click(button(view, en.removeConnection));
    assert.equal(requests.filter(({ path }) => path.endsWith('/remove')).length, 1);
    assert.ok(!view.textContent.includes(entries[0].label));
    assert.ok(view.textContent.includes(entries[1].label));
});

test('switching hosts aborts old log requests and ignores a late reply', async t => {
    const job = { id: 'same-id', action: 'build', status: 'succeeded', started: 1, finished: 2 };
    let finishLocalLog;
    const localLog = new Promise(resolve => { finishLocalLog = resolve; });
    const requests = mockApi(t, ({ path }) => {
        switch (path) {
            case 'hosts': return json([localHost, remoteHost]);
            case 'hosts/local/status': return json(status);
            case 'hosts/peer-1/status': return json({ ...status, host: 'remote' });
            case 'hosts/local/jobs':
            case 'hosts/peer-1/jobs': return json([job]);
            case 'hosts/local/jobs/same-id/log': return localLog;
            case 'hosts/peer-1/jobs/same-id/log': return new Response('remote log');
            default: assert.fail('Unexpected request ' + path);
        }
    });
    const view = await mount(t, createElement(App), { signedIn: true });
    await click(button(view, en.history));
    const request = requests.find(({ path }) => path === 'hosts/local/jobs/same-id/log');
    assert.ok(request);
    assert.equal(request.signal.aborted, false);

    await selectHost(view, remoteHost.id);
    assert.equal(request.signal.aborted, true);
    assert.ok(view.textContent.includes('remote log'));
    await act(async () => { finishLocalLog(new Response('stale local log')); });
    const log = element(view, '[role="region"][aria-label="' + en.operationLog + '"]');
    assert.equal(log.textContent, 'remote log');
});

test('opening sharing with pairing disabled does not mint a code until enabled', async t => {
    const requests = mockApi(t, ({ path, method, body }) => {
        if (path === 'connections') return json([]);
        if (path === 'pairings/settings') return json({ enabled: body.enabled });
        if (path === 'pairings/revoke') return json({ revoked: true });
        if (path === 'pairings') {
            if (method === 'GET') return json({ enabled: false });
            return json({ code: 'AB3K9Q', link: '~fixture#AB3K9Q' });
        }
        assert.fail('Unexpected request ' + path);
    });
    const session = { invite: null, receivedPairs: null };
    const view = await mount(t, connectionsPage(session));
    await click(element(view, '[role="button"][aria-label="' + en.addConnection + '"]'));
    assert.equal(session.invite, null);
    assert.ok(view.textContent.includes(en.pairingDisabled));
    assert.equal(button(view, en.copyPairingCode).disabled, true);
    assert.equal(requests.filter(({ method }) => method === 'POST').length, 0);

    await click(element(view, '#pairing-enabled'));
    assert.deepEqual(requests.filter(({ method }) => method === 'POST').map(({ path, body }) => [path, body]), [
        ['pairings/settings', { enabled: true }],
        ['pairings', {}],
    ]);
    assert.equal(session.invite.code, 'AB3K9Q');
    assert.equal(button(view, en.copyPairingCode).disabled, false);

    await click(element(view, '#pairing-enabled'));
    assert.deepEqual(requests.filter(({ path }) => path === 'pairings/settings').map(({ body }) => body), [
        { enabled: true },
        { enabled: false },
    ]);
    assert.equal(session.invite, null);
    assert.equal(button(view, en.copyPairingCode).disabled, true);
});
