import { useEffect, useRef, useState } from 'react';
import { Button, Card, Checkbox, Icon, Stack, Text, TextInput } from '@alpha/react';
import { api, ApiError, type Connection, type ConnectionCheck } from './model';

type View = 'list' | 'share' | 'join';

type Props = { token: string; gateway: string; t: (key: string) => string; onChange: () => Promise<void>; onAuthenticationError: (error: Error) => void };
export function ConnectionsPage({ token, gateway, t, onChange, onAuthenticationError }: Props) {
    const [connections, setConnections] = useState<Connection[]>([]);
    const [health, setHealth] = useState<Record<string, 'checking' | 'online' | 'offline'>>({});
    const [busy, setBusy] = useState<string | null>(null), [loading, setLoading] = useState(true);
    const [revision, setRevision] = useState(0);
    const [error, setError] = useState(''), [removing, setRemoving] = useState<string | null>(null);
    const controller = useRef(new AbortController());
    const [created, setCreated] = useState(''), [copied, setCopied] = useState(false);
    const [link, setLink] = useState(''), [code, setCode] = useState(''), [origin, setOrigin] = useState(''), [allowPublic, setAllowPublic] = useState(false), [paired, setPaired] = useState(false);
    const [view, setView] = useState<View>('list');
    const [advanced, setAdvanced] = useState(false);
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
    function pairingFail(value: unknown) {
        if (controller.current.signal.aborted) return;
        const failure = value instanceof Error ? value : new Error(String(value));
        if (failure instanceof ApiError && failure.status === 401) onAuthenticationError(failure);
        else setError(t('pairingFailed') + ': ' + failure.message);
    }
    async function createLink(signal: AbortSignal) {
        setBusy('pair'); setError(''); setCreated(''); setCopied(false);
        try {
            const reply = await api<{code: string; link: string}>(token, 'pairings', signal, {});
            if (signal.aborted) return;
            setCreated(reply.link);
        } catch (e) { pairingFail(e); }
        finally { if (!signal.aborted) setBusy(null); }
    }
    function splitIncoming(value: string) {
        const text = value.trim();
        if (!text.includes('#')) {
            setLink(text); setPaired(false); setError('');
            return;
        }
        const head = text.slice(0, text.indexOf('#'));
        const tail = text.slice(text.indexOf('#') + 1).trim().replace(/\D/g, '').slice(0, 6);
        setLink(head); setCode(tail); setPaired(false); setError('');
    }
    async function acceptLink() {
        const cleanLink = link.trim(), cleanCode = code.trim();
        if (busy || (!cleanLink && !cleanCode)) return;
        const body = cleanLink
            ? (cleanCode ? cleanLink + '#' + cleanCode : cleanLink)
            : cleanCode;
        setBusy('accept'); setError(''); setPaired(false);
        try {
            const entry = await api<Connection>(token, 'pairings/accept', controller.current.signal, { link: body, origin: origin.trim() || undefined, allow_public: allowPublic });
            if (controller.current.signal.aborted) return;
            setConnections(current => [...current, entry]);
            setHealth(current => ({ ...current, [entry.id]: 'online' }));
            setLink(''); setCode(''); setOrigin(''); setAllowPublic(false); setPaired(true);
            await onChange();
            setRevision(value => value + 1);
            setView('list');
        } catch (e) { pairingFail(e); }
        finally { if (!controller.current.signal.aborted) setBusy(null); }
    }
    async function copyLink() {
        try { await navigator.clipboard.writeText(created); setCopied(true); }
        catch { setCopied(false); }
    }
    function goToShare() {
        setError(''); setCopied(false);
        setView('share');
        if (!created) {
            const active = new AbortController();
            void createLink(active.signal);
        }
    }
    function goToJoin() {
        setError(''); setLink(''); setCode(''); setOrigin(''); setAllowPublic(false); setPaired(false); setAdvanced(false);
        setView('join');
    }
    function goBack() {
        setError('');
        setView(view === 'join' ? 'share' : 'list');
    }

    // --- View: Share link ---
    if (view === 'share') {
        return <Stack gap={5}>
            <Card><Stack gap={4}>
                <Stack gap={2}>
                    <Text as="h2" font="heading" size="title" weight="medium">{t('pairingLink')}</Text>
                    <Text tone="secondary">{t('pairingShareHelp')}</Text>
                </Stack>
                <div className="pairing-link-display" onClick={() => void copyLink()}>
                    <Text font="mono">{created || t('connectingShort')}</Text>
                </div>
                {copied && <div role="status"><Text tone="secondary">{t('pairingLinkCopied')}</Text></div>}
                <div className="button-row">
                    <Button variant="simple" onPress={goBack}>{t('back')}</Button>
                    <Button variant="simple" disabled={!!busy || !created} onPress={goToJoin}>{t('addOtherInstance')}</Button>
                </div>
            </Stack></Card>
            {error && <div role="alert"><Card><Stack gap={3}><Text weight="medium">{t('connectionFailed')}</Text><Text>{error}</Text></Stack></Card></div>}
        </Stack>;
    }

    // --- View: Join / add another instance ---
    if (view === 'join') {
        return <Stack gap={5}>
            <Card><Stack gap={4}>
                <div className="button-row"><Button variant="simple" onPress={goBack}><Icon name="back" />{t('back')}</Button></div>
                <div className="pairing-row">
                    <TextInput id="pairing-link" label={t('pairingPasteLink')} labelPlacement="above" showIcon={false} placeholder={t('pairingLinkPlaceholder')} value={link} onValueChange={value => splitIncoming(value)} />
                    <TextInput id="pairing-code" label={t('pairingCode')} labelPlacement="above" placeholder={t('pairingCodePlaceholder')} value={code} onValueChange={value => { setCode(value.replace(/\D/g, '').slice(0, 6)); setPaired(false); setError(''); }} />
                </div>
                <div className="button-row"><Button disabled={!!busy || (!link.trim() && !code.trim())} loading={busy === 'accept'} onPress={() => void acceptLink()}>{t('pairingAccept')}</Button></div>
                {paired && <div role="status"><Text>{t('pairingPaired')}</Text></div>}
                <div className={`advanced-toggle ${advanced ? 'advanced-open' : ''}`} onClick={() => setAdvanced(!advanced)}>
                    <Icon name="chevron" />
                    <Text tone="secondary">{t('advanced')}</Text>
                </div>
                {advanced && <Stack gap={4}>
                    <TextInput id="pairing-origin" label={t('pairingOrigin')} labelPlacement="above" type="url" placeholder={t('pairingOriginPlaceholder')} value={origin} onValueChange={value => { setOrigin(value); setPaired(false); setError(''); }} />
                    <Checkbox id="pairing-public" label={t('pairingPublicConfirm')} hint={t('pairingPublicHelp')} checked={allowPublic} onCheckedChange={setAllowPublic} />
                </Stack>}
            </Stack></Card>
            {error && <div role="alert"><Card><Stack gap={3}><Text weight="medium">{t('connectionFailed')}</Text><Text>{error}</Text></Stack></Card></div>}
        </Stack>;
    }

    // --- View: List (default) ---
    return <Stack gap={5}>
        <div className="connections-grid">
            <Card><Stack gap={3}><Text as="h2" font="heading" size="title" weight="medium">{gateway}</Text><Text tone="secondary">{t('connectionOnline')}</Text></Stack></Card>
            {connections.map(entry => <Card key={entry.id}><Stack gap={4}>
                <div className="path-value"><Text as="h2" font="heading" size="title" weight="medium">{entry.label}</Text><Text font="mono">{entry.url}</Text></div>
                <div role="status"><Text tone="secondary">{t(health[entry.id] === 'online' ? 'connectionOnline' : health[entry.id] === 'offline' ? 'connectionOffline' : 'connectingShort')}</Text></div>
                {entry.managed_by_nix && <Text tone="muted">{t('connectionManagedByNix')}</Text>}
                {removing === entry.id ? <Stack gap={3}><Text>{t('removeConnectionHelp')}</Text><div className="button-row"><Button variant="simple" disabled={!!busy} onPress={() => setRemoving(null)}>{t('cancel')}</Button><Button disabled={!!busy} loading={busy === entry.id} onPress={() => void remove(entry.id)}>{t('removeConnection')}</Button></div></Stack> : <div className="button-row"><Button variant="simple" disabled={!!busy || health[entry.id] === 'checking'} onPress={() => void checkSaved(entry.id, controller.current.signal)}>{t('testConnection')}</Button>{!entry.managed_by_nix && <Button variant="simple" disabled={!!busy} onPress={() => setRemoving(entry.id)}>{t('removeConnection')}</Button>}</div>}
            </Stack></Card>)}
            <div className="add-connection-card" onClick={goToShare}><Card><Stack gap={3} align="center">
                <Icon name="add" />
                <Text as="h2" font="heading" size="title" weight="medium">{t('addConnection')}</Text>
                <Text tone="secondary">{t('addConnectionHelp')}</Text>
            </Stack></Card></div>
        </div>
        {error && <div role="alert"><Card><Stack gap={3}><Text weight="medium">{t('connectionFailed')}</Text><Text>{error}</Text></Stack></Card></div>}
        {loading && <div role="status"><Text>{t('connectingShort')}</Text></div>}
    </Stack>;
}
