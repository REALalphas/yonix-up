import { useEffect, useRef, useState } from 'react';
import { Button, Card, Checkbox, Icon, Stack, Text, TextInput } from '@alpha/react';
import { api, ApiError, type Connection, type ConnectionCheck } from './model';

type PairingView = 'closed' | 'share' | 'join';
const pairingLifetimeMs = 10 * 60 * 1000;
type PairingInvite = { link: string; expiresAt: number };
export type PairingSession = {
    invite: PairingInvite | null;
    receivedPairs: string[] | null;
};

type Props = { token: string; gateway: string; pairingSession: PairingSession; t: (key: string) => string; onChange: () => Promise<void>; onAuthenticationError: (error: Error) => void };
export function ConnectionsPage({ token, gateway, pairingSession, t, onChange, onAuthenticationError }: Props) {
    const [connections, setConnections] = useState<Connection[]>([]);
    const [health, setHealth] = useState<Record<string, 'checking' | 'online' | 'offline'>>({});
    const [busy, setBusy] = useState<string | null>(null), [loading, setLoading] = useState(true);
    const [revision, setRevision] = useState(0);
    const [error, setError] = useState(''), [removing, setRemoving] = useState<string | null>(null);
    const controller = useRef(new AbortController());
    const [created, setCreated] = useState(pairingSession.invite);
    const [pairingError, setPairingError] = useState('');
    const [incoming, setIncoming] = useState(''), [manualCode, setManualCode] = useState<string | null>(null);
    const [origin, setOrigin] = useState(''), [allowPublic, setAllowPublic] = useState(false), [paired, setPaired] = useState(false);
    const [view, setView] = useState<PairingView>('closed');
    const previousView = useRef(view);
    const pairingNavigation = useRef<HTMLDivElement>(null);
    const addCardKey = useRef<string | null>(null);
    const pairingSuccess = useRef<HTMLDivElement>(null);
    const advanced = useRef<HTMLDetailsElement>(null);
    const [link, linkCode = ''] = incoming.split('#', 2);
    const code = manualCode ?? linkCode;
    const canAccept = code.length === 6 && !!(link || origin.trim());
    const sharedLink = created && created.expiresAt > Date.now() ? created.link : '';

    useEffect(() => {
        if (view === previousView.current) return;
        previousView.current = view;
        const navigation = pairingNavigation.current;
        const target = pairingSuccess.current ?? navigation?.querySelector<HTMLButtonElement>('button:not(:disabled)') ?? navigation;
        target?.focus();
    }, [view, paired]);
    function updateInvite(invite: PairingInvite | null) {
        pairingSession.invite = invite;
        setCreated(invite);
    }
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
                // The initiator saves incoming redeems as pair-* entries.
                const received = entries.filter(entry => !entry.managed_by_nix && entry.id.startsWith('pair-')).map(entry => entry.id);
                const previous = pairingSession.receivedPairs;
                pairingSession.receivedPairs = received;
                if (previous && received.some(id => !previous.includes(id))) {
                    updateInvite(null);
                }
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
    function pairingFail(value: unknown, show = true) {
        const failure = value instanceof Error ? value : new Error(String(value));
        if (failure instanceof ApiError && failure.status === 401) onAuthenticationError(failure);
        else if (show) setPairingError(t('pairingFailed') + ': ' + failure.message);
    }
    async function createLink(signal: AbortSignal) {
        if (busy) return;
        const expiresAt = Date.now() + pairingLifetimeMs;
        setBusy('pair'); setPairingError(''); updateInvite(null);
        try {
            const reply = await api<{ link: string }>(token, 'pairings', signal, {});
            if (signal.aborted) return;
            updateInvite({ link: reply.link, expiresAt });
        } catch (e) { if (!signal.aborted) pairingFail(e, previousView.current === 'share'); }
        finally { if (!signal.aborted) setBusy(null); }
    }
    useEffect(() => {
        if (view !== 'share' || pairingError || busy) return;
        if (!created || created.expiresAt <= Date.now()) {
            void createLink(controller.current.signal);
            return;
        }
        const timer = setTimeout(() => void createLink(controller.current.signal), Math.max(0, created.expiresAt - Date.now()));
        return () => clearTimeout(timer);
    }, [view, created, pairingError, busy]);
    function updateIncoming(value: string) {
        const [head, fragment] = value.trim().split('#', 2);
        if (fragment !== undefined) {
            setIncoming(head.trim() + '#' + fragment.replace(/\D/g, '').slice(0, 6));
            setManualCode(null);
        } else if (/^\d{6}$/.test(head)) {
            setIncoming(''); setManualCode(head);
            if (advanced.current) advanced.current.open = true;
        } else {
            setIncoming(head);
        }
        setPairingError('');
    }
    async function acceptLink() {
        if (busy || !canAccept) return;
        const signal = controller.current.signal;
        const body = link ? link + '#' + code : code;
        setBusy('accept'); setPairingError(''); setPaired(false);
        try {
            const entry = await api<Connection>(token, 'pairings/accept', signal, { link: body, origin: origin.trim() || undefined, allow_public: allowPublic });
            if (signal.aborted) return;
            setConnections(current => current.some(saved => saved.id === entry.id) ? current : [...current, entry]);
            setHealth(current => ({ ...current, [entry.id]: 'online' }));
            setPaired(true);
            setView('closed');
            try { await onChange(); }
            catch (e) { if (!signal.aborted) fail(e); }
            if (!signal.aborted) setRevision(value => value + 1);
        } catch (e) { if (!signal.aborted) pairingFail(e); }
        finally { if (!signal.aborted) setBusy(null); }
    }
    async function copyLink() {
        if (!sharedLink) return;
        const signal = controller.current.signal;
        if (navigator.clipboard) {
            try {
                await navigator.clipboard.writeText(sharedLink);
                return;
            } catch { /* Clipboard access may be denied. */ }
        }
        // Plain HTTP origins do not have access to the async Clipboard API.
        if (signal.aborted || previousView.current !== 'share' || pairingSession.invite?.link !== sharedLink) return;
        function writeLink(event: ClipboardEvent) {
            if (!event.clipboardData) return;
            event.clipboardData.setData('text/plain', sharedLink);
            event.preventDefault();
        }
        document.addEventListener('copy', writeLink);
        try { if (document.execCommand('copy')) return; }
        catch { /* Leave the link selected for manual copying. */ }
        finally { document.removeEventListener('copy', writeLink); }
        const field = document.getElementById('pairing-share-link');
        if (field instanceof HTMLInputElement) { field.focus(); field.select(); }
    }
    function goToShare() {
        addCardKey.current = null;
        setPaired(false); setPairingError('');
        setView('share');
    }
    function goToJoin() {
        setPairingError(''); setIncoming(''); setManualCode(null); setOrigin(''); setAllowPublic(false);
        setView('join');
    }
    const pairingAlert = pairingError && <div role="alert"><Text>{pairingError}</Text></div>;
    return <Stack gap={5}>
        <div className="connections-grid">
            <Card><Stack gap={3}><Text as="h2" font="heading" size="title" weight="medium">{gateway}</Text><Text tone="secondary">{t('connectionOnline')}</Text></Stack></Card>
            {connections.map(entry => <Card key={entry.id}><Stack gap={4}>
                <div className="path-value"><Text as="h2" font="heading" size="title" weight="medium">{entry.label}</Text><Text font="mono">{entry.url}</Text></div>
                <div role="status"><Text tone="secondary">{t(health[entry.id] === 'online' ? 'connectionOnline' : health[entry.id] === 'offline' ? 'connectionOffline' : 'connectingShort')}</Text></div>
                {entry.managed_by_nix && <Text tone="muted">{t('connectionManagedByNix')}</Text>}
                {removing === entry.id ? <Stack gap={3}><Text>{t('removeConnectionHelp')}</Text><div className="button-row"><Button variant="simple" disabled={!!busy} onPress={() => setRemoving(null)}>{t('cancel')}</Button><Button disabled={!!busy} loading={busy === entry.id} onPress={() => void remove(entry.id)}>{t('removeConnection')}</Button></div></Stack> : <div className="button-row"><Button variant="simple" disabled={!!busy || health[entry.id] === 'checking'} onPress={() => void checkSaved(entry.id, controller.current.signal)}>{t('testConnection')}</Button>{!entry.managed_by_nix && <Button variant="simple" disabled={!!busy} onPress={() => setRemoving(entry.id)}>{t('removeConnection')}</Button>}</div>}
            </Stack></Card>)}
            {view === 'closed' ? <div ref={pairingNavigation} role="button" tabIndex={0}
                aria-label={t('addConnection')} aria-describedby="add-connection-help" onClick={goToShare}
                onBlur={() => { addCardKey.current = null; }}
                onKeyDown={event => {
                    if (event.key !== ' ' && event.key !== 'Enter') return;
                    event.preventDefault();
                    addCardKey.current = event.key;
                }}
                onKeyUp={event => { if (event.key === addCardKey.current) goToShare(); }}>
                <Card><Stack gap={3}>
                    <Stack direction="row" align="center" gap={2}>
                        <Icon name="add" />
                        <Text as="h2" font="heading" size="title" weight="medium">{t('addConnection')}</Text>
                    </Stack>
                    <div id="add-connection-help"><Text tone="secondary">{t('addConnectionHelp')}</Text></div>
                </Stack></Card>
            </div> : <Card>
                {view === 'share' ? <Stack gap={4}>
                    <div ref={pairingNavigation} tabIndex={-1}>
                        <Button variant="simple" onPress={() => setView('closed')}>{t('back')}</Button>
                    </div>
                    <TextInput id="pairing-share-link" label={t('pairingLink')} labelPlacement="above" autoComplete="off" showIcon={false} readOnly value={sharedLink} />
                    {!sharedLink && !pairingError && <div role="status"><Text>{t('connectingShort')}</Text></div>}
                    {pairingAlert}
                    <div className="pairing-share-actions">
                        <Button variant="simple" width="container" disabled={!sharedLink} onPress={() => void copyLink()}>{t('copyPairingLink')}</Button>
                        <Button variant="simple" width="container" onPress={goToJoin}>{t('addOtherInstance')}</Button>
                    </div>
                </Stack> : <form noValidate onSubmit={event => { event.preventDefault(); void acceptLink(); }}><Stack gap={4}>
                    <div ref={pairingNavigation} tabIndex={-1}>
                        <Button variant="simple" disabled={busy === 'accept'} onPress={goToShare}>{t('back')}</Button>
                    </div>
                    <Text as="h2" font="heading" size="title" weight="medium">{t('addOtherInstance')}</Text>
                    <div className="pairing-row">
                        <TextInput id="pairing-link" label={t('pairingPasteLink')} labelPlacement="above" showIcon={false} autoComplete="off" disabled={busy === 'accept'} value={link} onValueChange={updateIncoming} />
                        <TextInput id="pairing-code" label={t('pairingCode')} labelPlacement="above" type="tel" showIcon={false} autoComplete="off" placeholder={t('pairingCodePlaceholder')} disabled={busy === 'accept'} value={code} onValueChange={value => { setManualCode(value.replace(/\D/g, '').slice(0, 6)); setPairingError(''); }} />
                    </div>
                    <details ref={advanced}>
                        <summary><Text as="span">{t('advanced')}</Text></summary>
                        <Stack gap={4}>
                            <TextInput id="pairing-origin" label={t('pairingOrigin')} labelPlacement="above" type="url" placeholder={t('pairingOriginPlaceholder')} disabled={busy === 'accept'} value={origin} onValueChange={value => { setOrigin(value); setPairingError(''); }} />
                            <Checkbox id="pairing-public" label={t('pairingPublicConfirm')} disabled={busy === 'accept'} checked={allowPublic} onCheckedChange={value => { setAllowPublic(value); setPairingError(''); }} />
                        </Stack>
                    </details>
                    {pairingAlert}
                    <div className="button-row"><Button type="submit" disabled={!!busy || !canAccept} loading={busy === 'accept'}>{t('pairingAccept')}</Button></div>
                </Stack></form>}
            </Card>}
        </div>
        {error && <div role="alert"><Card><Stack gap={3}><Text weight="medium">{t('connectionFailed')}</Text><Text>{error}</Text></Stack></Card></div>}
        {paired && <div role="status" ref={pairingSuccess} tabIndex={-1}><Text>{t('pairingPaired')}</Text></div>}
        {loading && <div role="status"><Text>{t('connectingShort')}</Text></div>}
    </Stack>;
}
