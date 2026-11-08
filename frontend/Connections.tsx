import { useEffect, useRef, useState } from 'react';
import { Button, Card, Stack, Text, TextInput } from '@alpha/react';
import { api, ApiError, type Connection, type ConnectionCheck } from './model';

type Props = { token: string; gateway: string; t: (key: string) => string; onChange: () => Promise<void>; onAuthenticationError: (error: Error) => void };
export function ConnectionsPage({ token, gateway, t, onChange, onAuthenticationError }: Props) {
    const [connections, setConnections] = useState<Connection[]>([]);
    const [health, setHealth] = useState<Record<string, 'checking' | 'online' | 'offline'>>({});
    const [label, setLabel] = useState(''), [url, setUrl] = useState(''), [secret, setSecret] = useState('');
    const [visible, setVisible] = useState(false), [busy, setBusy] = useState<string | null>(null), [loading, setLoading] = useState(true);
    const [adding, setAdding] = useState(false), [revision, setRevision] = useState(0);
    const [error, setError] = useState(''), [checked, setChecked] = useState<ConnectionCheck | null>(null), [added, setAdded] = useState(false), [removing, setRemoving] = useState<string | null>(null);
    const controller = useRef(new AbortController());
    const valid = !!label.trim() && !!url.trim() && secret.trim().length === 64;
    function fail(value: unknown) {
        if (controller.current.signal.aborted) return;
        const failure = value instanceof Error ? value : new Error(String(value));
        if (failure instanceof ApiError && failure.status === 401) onAuthenticationError(failure);
        else setError(failure.message);
    }
    async function checkSaved(id: string, signal: AbortSignal) {
        setHealth(current => ({ ...current, [id]: 'checking' }));
        try {
            await api<ConnectionCheck>(token, 'connections/' + encodeURIComponent(id) + '/check', signal, {});
            if (!signal.aborted) setHealth(current => ({ ...current, [id]: 'online' }));
        } catch (e) {
            if (!signal.aborted) {
                setHealth(current => ({ ...current, [id]: 'offline' }));
                if (e instanceof ApiError && e.status === 401) onAuthenticationError(e);
            }
        }
    }
    useEffect(() => {
        const active = new AbortController(); controller.current = active;
        let timer: ReturnType<typeof setTimeout>;
        async function refresh() {
            try {
                const entries = await api<Connection[]>(token, 'connections', active.signal);
                if (active.signal.aborted) return;
                setConnections(entries); setLoading(false);
                await Promise.all(entries.map(entry => checkSaved(entry.id, active.signal)));
            } catch (e) { if (!active.signal.aborted) { setLoading(false); fail(e); } }
            finally { if (!active.signal.aborted) timer = setTimeout(refresh, 15000); }
        }
        void refresh();
        return () => { active.abort(); clearTimeout(timer); };
    }, [token, revision]);
    useEffect(() => {
        if (adding) document.getElementById('connection-label')?.focus();
    }, [adding]);
    function closeForm() {
        setAdding(false); setSecret(''); setLabel(''); setUrl(''); setVisible(false); setChecked(null);
        requestAnimationFrame(() => document.getElementById('add-instance')?.focus());
    }
    function change(set: (value: string) => void, value: string) { set(value); setChecked(null); setAdded(false); setError(''); }
    async function submit(save: boolean) {
        if (!valid || busy) return;
        setBusy(save ? 'save' : 'check'); setError(''); setAdded(false);
        try {
            const body = { label: label.trim(), url: url.trim(), token: secret.trim() };
            if (save) {
                const entry = await api<Connection>(token, 'connections', controller.current.signal, body);
                if (controller.current.signal.aborted) return;
                setConnections(current => [...current, entry]);
                setHealth(current => ({ ...current, [entry.id]: 'online' }));
                closeForm(); setAdded(true);
                await onChange();
                setRevision(value => value + 1);
            } else {
                const result = await api<ConnectionCheck>(token, 'connections/check', controller.current.signal, body);
                if (!controller.current.signal.aborted) setChecked(result);
            }
        } catch (e) { fail(e); }
        finally { if (!controller.current.signal.aborted) setBusy(null); }
    }
    async function remove(id: string) {
        if (busy) return;
        setBusy(id); setError('');
        try {
            await api(token, 'connections/' + encodeURIComponent(id) + '/remove', controller.current.signal, {});
            if (controller.current.signal.aborted) return;
            setConnections(current => current.filter(entry => entry.id !== id)); setRemoving(null);
            await onChange();
            setRevision(value => value + 1);
        } catch (e) { fail(e); }
        finally { if (!controller.current.signal.aborted) setBusy(null); }
    }
    return <Stack gap={5}>
        <div className="page-actions"><Button id="add-instance" disabled={adding} onPress={() => { setAdding(true); setAdded(false); }}>{t('addInstance')}</Button></div>
        {error && <div role="alert"><Card><Stack gap={3}><Text weight="medium">{t('connectionFailed')}</Text><Text>{error}</Text></Stack></Card></div>}
        {added && <div role="status"><Text>{t('connectionAdded')}</Text></div>}
        {adding && <Card><Stack gap={5}>
            <Text as="h2" font="heading" size="title" weight="medium">{t('addInstance')}</Text>
            <form onSubmit={event => { event.preventDefault(); void submit(true); }}><Stack gap={4}>
                <TextInput id="connection-label" labelPlacement="above" label={t('connectionName')} placeholder="Web" required disabled={!!busy} value={label} onValueChange={value => change(setLabel, value)}/>
                <TextInput id="connection-url" labelPlacement="above" label={t('connectionAddress')} type="url" placeholder="http://10.0.0.20:9847" required disabled={!!busy} value={url} onValueChange={value => change(setUrl, value)}/>
                <TextInput id="connection-token" labelPlacement="above" label={t('accessToken')} type="password" icon="key" required autoComplete="off" disabled={!!busy} value={secret} onValueChange={value => change(setSecret, value)} passwordVisible={visible} onPasswordVisibleChange={setVisible} hint={t('connectionTokenHelp')}/>
                {checked && <div role="status"><Text>{t('connectionVerified')} <Text as="span" font="mono">{checked.host} · Yonix {checked.version}</Text></Text></div>}
                <div className="button-row"><Button type="button" variant="simple" disabled={!!busy} onPress={closeForm}>{t('cancel')}</Button><Button type="button" variant="simple" disabled={!valid || !!busy} loading={busy === 'check'} onPress={() => void submit(false)}>{t('testConnection')}</Button><Button type="submit" disabled={!valid || !!busy} loading={busy === 'save'}>{t('addInstance')}</Button></div>
            </Stack></form>
        </Stack></Card>}
        <div className="connections-grid">
            <Card><Stack gap={3}><Text as="h2" font="heading" size="title" weight="medium">{gateway}</Text><Text tone="secondary">{t('connectionOnline')}</Text></Stack></Card>
            {connections.map(entry => <Card key={entry.id}><Stack gap={4}>
                <div className="path-value"><Text as="h2" font="heading" size="title" weight="medium">{entry.label}</Text><Text font="mono">{entry.url}</Text></div>
                <div role="status"><Text tone="secondary">{t(health[entry.id] === 'online' ? 'connectionOnline' : health[entry.id] === 'offline' ? 'connectionOffline' : 'connectingShort')}</Text></div>
                {entry.managed_by_nix && <Text tone="muted">{t('connectionManagedByNix')}</Text>}
                {removing === entry.id ? <Stack gap={3}><Text>{t('removeConnectionHelp')}</Text><div className="button-row"><Button variant="simple" disabled={!!busy} onPress={() => setRemoving(null)}>{t('cancel')}</Button><Button disabled={!!busy} loading={busy === entry.id} onPress={() => void remove(entry.id)}>{t('removeConnection')}</Button></div></Stack> : <div className="button-row"><Button variant="simple" disabled={!!busy || health[entry.id] === 'checking'} onPress={() => void checkSaved(entry.id, controller.current.signal)}>{t('testConnection')}</Button>{!entry.managed_by_nix && <Button variant="simple" disabled={!!busy} onPress={() => setRemoving(entry.id)}>{t('removeConnection')}</Button>}</div>}
            </Stack></Card>)}
        </div>
        {loading && <div role="status"><Text>{t('connectingShort')}</Text></div>}
    </Stack>;
}
