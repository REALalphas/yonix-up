import { useEffect, useRef, useState, type ClipboardEvent as ReactClipboardEvent } from 'react';
import { Button, Card, Checkbox, Icon, Stack, Text, TextInput } from '@alpha/react';
import { api, ApiError, type Connection, type ConnectionCheck } from './model';

type PairingView = 'closed' | 'share' | 'join';
const codeLength = 6;
const emptyCode = () => Array<string>(codeLength).fill('');
const normalizeCode = (value: string) => /^[0-9\s-]*$/.test(value) ? value.replace(/[\s-]/g, '') : '';
type PairingInvite = { code: string; link: string; expiresAt: number };
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
    const [incoming, setIncoming] = useState('');
    const [manualCopy, setManualCopy] = useState('');
    const [codeDigits, setCodeDigits] = useState(emptyCode);
    const [origin, setOrigin] = useState(''), [allowPublic, setAllowPublic] = useState(false), [paired, setPaired] = useState(false);
    const [view, setView] = useState<PairingView>('closed');
    const previousView = useRef(view);
    const pairingNavigation = useRef<HTMLDivElement>(null);
    const addCardKey = useRef<string | null>(null);
    const pairingSuccess = useRef<HTMLDivElement>(null);
    const codeEntry = useRef<HTMLDivElement>(null);
    const [link] = incoming.trim().split('#', 2);
    const displayedCode = codeDigits.join('');
    const canAccept = displayedCode.length === codeLength;
    const invite = created && created.expiresAt > Date.now() ? created : null;

    useEffect(() => {
        if (view === previousView.current) return;
        previousView.current = view;
        const navigation = pairingNavigation.current;
        const target = pairingSuccess.current ?? codeEntry.current?.querySelector('input') ?? navigation?.querySelector<HTMLButtonElement>('button:not(:disabled)') ?? navigation;
        target?.focus();
    }, [view, paired]);
    function updateInvite(invite: PairingInvite | null) {
        pairingSession.invite = invite;
        setCreated(invite);
        setManualCopy('');
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
                if (previous && pairingSession.invite && received.some(id => !previous.includes(id))) {
                    updateInvite(null);
                    if (previousView.current === 'share') {
                        setView('closed');
                        setPaired(true);
                        await onChange();
                    }
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
        setBusy('pair'); setPairingError(''); updateInvite(null);
        try {
            const reply = await api<{ code: string; link: string; expires_in?: number }>(token, 'pairings', signal, {});
            if (signal.aborted) return;
            updateInvite({ code: reply.code, link: reply.link, expiresAt: Date.now() + (reply.expires_in ?? 600) * 1000 });
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
    function focusCode(index: number) {
        codeEntry.current?.querySelectorAll('input')[Math.min(Math.max(index, 0), codeLength - 1)]?.focus();
    }
    function writeCode(value: string, index: number = 0) {
        const digits = normalizeCode(value);
        if (value && (!digits || digits.length > codeLength)) return;
        const start = digits.length === codeLength ? 0 : index;
        setCodeDigits(current => {
            const next = [...current];
            next.splice(start, digits.length || 1, ...(digits ? [...digits].slice(0, codeLength - start) : ['']));
            return next;
        });
        if (digits) focusCode(start + digits.length);
        setPairingError('');
    }
    function updateIncoming(value: string) {
        const [head, fragment] = value.trim().split('#', 2);
        const isCode = fragment === undefined && normalizeCode(head).length === codeLength;
        setIncoming(isCode ? '' : value);
        if (isCode || fragment !== undefined) {
            const digits = normalizeCode(isCode ? head : fragment);
            setCodeDigits(emptyCode().map((_, index) => digits.length <= codeLength ? digits[index] ?? '' : ''));
        }
        setPairingError('');
    }
    function pasteCode(event: ReactClipboardEvent<HTMLInputElement>, index: number) {
        const value = event.clipboardData.getData('text').trim();
        event.preventDefault();
        if (value.includes('#')) {
            updateIncoming(value);
            focusCode(codeLength - 1);
        } else {
            if (normalizeCode(value).length === codeLength) setIncoming('');
            writeCode(value, index);
        }
    }
    async function acceptLink() {
        if (busy || !canAccept) return;
        const signal = controller.current.signal;
        const body = link ? link + '#' + displayedCode : displayedCode;
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
    async function copyText(value: string) {
        if (!value) return;
        if (navigator.clipboard) {
            try { await navigator.clipboard.writeText(value); return; }
            catch { /* The browser can deny access on some origins. */ }
        }
        if (controller.current.signal.aborted || previousView.current !== 'share' || pairingSession.invite !== invite) return;
        const field = document.createElement('textarea');
        field.value = value;
        field.readOnly = true;
        field.className = 'pairing-copy-buffer';
        document.body.append(field);
        const previous = document.activeElement;
        field.select();
        try {
            if (!document.execCommand('copy')) setManualCopy(value);
        } catch {
            setManualCopy(value);
        } finally {
            field.remove();
            if (previous instanceof HTMLElement) previous.focus();
        }
    }
    function goToShare() {
        addCardKey.current = null;
        setManualCopy('');
        setPaired(false); setPairingError('');
        setView('share');
    }
    function goToJoin() {
        setPairingError(''); setIncoming(''); setCodeDigits(emptyCode()); setOrigin(''); setAllowPublic(false);
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
                    <div key={invite?.code} className="pairing-code-grid pairing-code-display" role="group" aria-label={t('pairingCode')}>
                        {emptyCode().map((_, index) => <input key={index} className="pairing-code-field"
                            aria-label={`${t('pairingCode')} ${index + 1}`} readOnly value={invite?.code[index] ?? ''}
                            disabled={!invite}
                            onFocus={event => event.currentTarget.select()}
                            onCopy={event => {
                                if (!invite) return;
                                event.preventDefault();
                                event.clipboardData.setData('text/plain', invite.code);
                            }} />)}
                    </div>
                    {!invite && !pairingError && <div role="status"><Text>{t('connectingShort')}</Text></div>}
                    {pairingAlert}
                    <div className="pairing-share-actions">
                        <Button variant="simple" width="container" disabled={!invite} onPress={() => void copyText(invite?.code ?? '')}>{t('copyPairingCode')}</Button>
                        <Button variant="simple" width="container" onPress={goToJoin}>{t('addOtherInstance')}</Button>
                    </div>
                    {manualCopy && <input aria-label={t(manualCopy === invite?.code ? 'pairingCode' : 'pairingLink')}
                        readOnly value={manualCopy} ref={field => { if (field) { field.focus(); field.select(); } }} />}
                    {invite?.link && <details>
                        <summary><Text as="span">{t('advanced')}</Text></summary>
                        <Stack gap={4}>
                            <TextInput id="pairing-share-link" label={t('pairingLink')} labelPlacement="above" autoComplete="off" showIcon={false} readOnly value={invite.link} />
                            <Button variant="simple" onPress={() => void copyText(invite.link)}>{t('copyPairingLink')}</Button>
                        </Stack>
                    </details>}
                </Stack> : <form noValidate onSubmit={event => { event.preventDefault(); void acceptLink(); }}><Stack gap={4}>
                    <div ref={pairingNavigation} tabIndex={-1}>
                        <Button variant="simple" disabled={busy === 'accept'} onPress={goToShare}>{t('back')}</Button>
                    </div>
                    <Text as="h2" font="heading" size="title" weight="medium">{t('addOtherInstance')}</Text>
                    <div ref={codeEntry} className="pairing-code-grid" role="group" aria-label={t('pairingCode')}>
                        {codeDigits.map((digit, index) => <input key={index} className="pairing-code-field"
                            aria-label={`${t('pairingCode')} ${index + 1}`} inputMode="numeric"
                            autoComplete={index === 0 ? 'one-time-code' : 'off'} disabled={busy === 'accept'} value={digit}
                            onFocus={event => event.currentTarget.select()}
                            onChange={event => writeCode(event.currentTarget.value, index)}
                            onPaste={event => pasteCode(event, index)}
                            onKeyDown={event => {
                                if (event.key === 'Backspace' && !digit && index > 0) {
                                    event.preventDefault();
                                    writeCode('', index - 1);
                                    focusCode(index - 1);
                                } else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
                                    event.preventDefault();
                                    focusCode(index + (event.key === 'ArrowLeft' ? -1 : 1));
                                }
                            }} />)}
                    </div>
                    <details>
                        <summary><Text as="span">{t('advanced')}</Text></summary>
                        <Stack gap={4}>
                            <TextInput id="pairing-link" label={t('pairingPasteLink')} labelPlacement="above" showIcon={false} autoComplete="off" disabled={busy === 'accept'} value={incoming} onValueChange={updateIncoming} />
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
