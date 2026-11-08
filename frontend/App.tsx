import { useEffect, useRef, useState } from 'react';
import { AppShell, Badge, Button, Card, Icon, Select, Stack, Text, TextInput } from '@alpha/react';
import { api, ApiError, allowed, actionKeys, confirmations, storedToken, saveToken, clearToken, type Action, type Job, type Status, type Host, hostPath } from './model';
import { ConnectionsPage } from './Connections';
import { HistoryPage } from './History';
import { catalogs, names, resolveLanguage, storedLanguage } from './i18n';
export function App() {
    const [preference, setPreference] = useState(storedLanguage), [browserLanguages, setBrowserLanguages] = useState([...navigator.languages]);
    const language = resolveLanguage(preference, browserLanguages), t = (key: string) => catalogs[language][key] ?? catalogs.en[key] ?? key;
    const [token, setToken] = useState(storedToken), [input, setInput] = useState(''), [status, setStatus] = useState<Status | null>(null), [jobs, setJobs] = useState<Job[]>([]), [selected, setSelected] = useState<string | null>(null), [log, setLog] = useState(''), [error, setError] = useState<Error | null>(null), [pending, setPending] = useState(false), [confirmation, setConfirmation] = useState<Action | null>(null), [page, setPage] = useState('system');
    const session = useRef(0);
    const newestJob = useRef<string | null>(null);
    const [hosts, setHosts] = useState<Host[]>([]);
    const [host, setHost] = useState('local');
    const hostRef = useRef(host); hostRef.current = host;
    const selectedHost = hosts.find(item => item.id === host);
    const gateway = hosts.find(item => item.local)?.label;
    const hostApi = <T,>(path: string, signal?: AbortSignal, body?: unknown) => api<T>(token, hostPath(host, path), signal, body);
    function switchHost(value: string) {
        if (pending || value === host) return;
        session.current++; hostRef.current = value;
        newestJob.current = null;
        setHost(value); setStatus(null); setJobs([]); setSelected(null); setLog(''); setConfirmation(null); setError(null);
    }
    const [revision, setRevision] = useState(0);
    const [passwordVisible, setPasswordVisible] = useState(false);
    useEffect(() => { document.documentElement.lang = language; }, [language]);
    useEffect(() => { const listener = () => setBrowserLanguages([...navigator.languages]); window.addEventListener('languagechange', listener); return () => window.removeEventListener('languagechange', listener); }, []);
    useEffect(() => {
        if (!token)
            return;
        let timer: ReturnType<typeof setTimeout>;
        const controller = new AbortController();
        const epoch = ++session.current;
        async function refresh() { try {
            const [nextStatus, nextJobs, nextHosts] = await Promise.all([hostApi<Status>('status', controller.signal), hostApi<Job[]>('jobs', controller.signal), api<Host[]>(token, 'hosts', controller.signal)]);
            if (session.current !== epoch)
                return;
            setStatus(nextStatus);
            setJobs(nextJobs);
            setHosts(nextHosts);
            setError(null);
            const previousNewest = newestJob.current, nextNewest = nextJobs[0]?.id ?? null;
            newestJob.current = nextNewest;
            setSelected(current => !current || current === previousNewest || !nextJobs.some(job => job.id === current) ? nextNewest : current);
        }
        catch (e) {
            if (!controller.signal.aborted && session.current === epoch) {
                setStatus(null);
                reportError(e);
            }
        }
        finally {
            if (!controller.signal.aborted)
                timer = setTimeout(refresh, 1500);
        } }
        void refresh();
        return () => { session.current++; controller.abort(); clearTimeout(timer); };
    }, [token, host, revision]);
    useEffect(() => {
        setLog('');
        if (!token || !selected || page !== 'history')
            return;
        const controller = new AbortController();
        let timer: ReturnType<typeof setTimeout>;
        async function refresh() { try {
            const text = await hostApi<string>('jobs/' + encodeURIComponent(selected!) + '/log', controller.signal);
            if (!controller.signal.aborted && hostRef.current === host)
                setLog(text);
        }
        catch (e) {
            if (!controller.signal.aborted)
                reportError(e);
        }
        finally {
            if (!controller.signal.aborted)
                timer = setTimeout(refresh, 1500);
        } }
        void refresh();
        return () => { controller.abort(); clearTimeout(timer); };
    }, [token, host, selected, page]);
    function logout() { session.current++; clearToken(); newestJob.current = null; setToken(''); setStatus(null); setJobs([]); setSelected(null); setLog(''); setConfirmation(null); setError(null); setInput(''); setPending(false); setHosts([]); setHost('local'); setPage('system'); }
    function reportError(value: unknown) {
        const failure = value instanceof Error ? value : new Error(String(value));
        if (failure instanceof ApiError && failure.status === 401) logout();
        setError(failure);
    }
    async function connect() { if (!input.trim() || pending)
        return; setPending(true); setError(null); const epoch = session.current; try {
        const value = input.trim();
        const [next, availableHosts] = await Promise.all([api<Status>(value, 'status'), api<Host[]>(value, 'hosts')]);
        if (session.current !== epoch)
            return;
        setHosts(availableHosts);
        setHost('local');
        setStatus(next);
        setToken(value);
        if (!saveToken(value)) setError(new Error(t('tokenStorageError')));
        setInput('');
    }
    catch (e) {
        if (session.current === epoch) reportError(e);
    }
    finally {
        setPending(false);
    } }
    async function start(action: Action, confirmed = false) { if (!allowed(action, status, pending))
        return; if (confirmations[action] && !confirmed) {
        setConfirmation(action);
        return;
    } setPending(true); setError(null); const epoch = session.current; try {
        const job = await hostApi<Job>('jobs', undefined, { action, confirmed });
        if (session.current !== epoch)
            return;
        setStatus(current => current ? { ...current, active: job.id } : current);
        setSelected(job.id);
        setPage('history');
        setConfirmation(null);
        setRevision(n => n + 1);
    }
    catch (e) {
        if (session.current === epoch)
            reportError(e);
    }
    finally {
        if (session.current === epoch)
            setPending(false);
    } }
    const languageSelect = <Select id="language" label={t('language')} value={preference} options={[{ value: 'system', label: t('systemLanguage') }, ...Object.keys(names).map(value => ({ value, label: names[value] }))]} onValueChange={value => { setPreference(value); try {
        localStorage.setItem('yonix-language', value);
    }
    catch { } }}/>;
    const authenticationFailed = error instanceof ApiError && error.status === 401;
    const errorView = error ? <div role="alert"><Card><Stack gap={2}><Text weight="medium">{t(authenticationFailed ? 'authenticationError' : 'connectionError')}</Text><Text font={authenticationFailed ? 'ui' : 'mono'}>{authenticationFailed ? t('authenticationHelp') : error.message}</Text></Stack></Card></div> : null;
    function operation(action: Action, description?: string) { return <Card key={action}><Stack gap={4}><Icon name={action === 'check' ? 'shield' : action === 'build' ? 'download' : action === 'diff' ? 'search' : action === 'switch' ? 'bolt' : action === 'update-inputs' ? 'refresh' : action === 'rollback' ? 'back' : 'settings'}/><Text as="h3" size="xl" font="heading" weight="medium">{t(actionKeys[action])}</Text>{description && <Text tone="secondary">{t(description)}</Text>}<div className="action-bottom"><Button width="container" disabled={!allowed(action, status, pending)} loading={pending && confirmation === action} onPress={() => void start(action)}>{t(actionKeys[action])}</Button></div></Stack></Card>; }
    if (!token)
        return <div className="alpha-ui login-layout"><div className="login-content"><Stack gap={7}><Stack gap={3}><Text font="heading" size="title" weight="medium">Yonix</Text><Text tone="secondary">{t('companion')}</Text></Stack><Card><Stack gap={6}><Stack gap={3}><Text as="h1" size="heading" font="heading" weight="medium">{t('welcome')}</Text><Text tone="secondary">{t('welcomeDescription')}</Text></Stack><form onSubmit={event => { event.preventDefault(); void connect(); }}><Stack gap={4}><TextInput id="token" label={t('accessToken')} type="password" passwordVisible={passwordVisible} onPasswordVisibleChange={setPasswordVisible} icon="key" value={input} onValueChange={setInput} placeholder={t('tokenPlaceholder')} autoComplete="off"/><Button type="submit" width="container" loading={pending} disabled={!input.trim()}>{t(pending ? 'connectingShort' : 'connect')}</Button></Stack></form><Text size="sm" tone="muted">{t('tokenCommand')} <Text as="span" font="mono">yonix token</Text> {t('onHost')}</Text></Stack></Card>{errorView}{languageSelect}</Stack></div></div>;
    return <div className="alpha-ui application-layout"><AppShell width="container" serviceName="Yonix" context={{ label: t('managedHost'), value: host, options: hosts.map(item => ({ value: item.id, label: item.label })), placeholder: '', disabled: pending }} onContextChange={switchHost} selectedId={page} items={[{ id: 'system', label: t('system'), icon: 'home' }, { id: 'history', label: t('history'), icon: 'queue' }, { id: 'connections', label: t('connections'), icon: 'link' }, { id: 'maintenance', label: t('maintenance'), icon: 'settings' }]} onSelect={id => { setPage(id); setConfirmation(null); }} onServicePress={() => setPage('system')} onAccountPress={() => setPage('settings')} accountState={error ? 'error' : status?.active ? 'loading' : 'default'}><div className="page-layout"><Text tone="secondary">{t(!status ? 'connectingShort' : status.active ? 'working' : 'ready')}</Text>{errorView}
 {confirmation && <div role="alert"><Card><Stack gap={4}><Text as="h2" size="title" font="heading">{t(actionKeys[confirmation])}</Text><Text>{t('targetHost')}: {selectedHost?.label}</Text><Text>{t(confirmations[confirmation]!)}</Text><div className="button-row"><Button variant="simple" disabled={pending} onPress={() => setConfirmation(null)}>{t('cancel')}</Button><Button disabled={!allowed(confirmation, status, pending)} loading={pending} onPress={() => void start(confirmation, true)}>{t('continue')}</Button></div></Stack></Card></div>}
 {page === 'system' && <><Stack gap={3}><Text as="h2" size="title" font="heading" weight="medium">{t('whatToDo')}</Text><Text tone="secondary">{t('checkingSafe')}</Text></Stack><div className="actions-grid">{operation('check', 'checkDescription')}{operation('build', 'buildDescription')}{operation('diff', 'diffDescription')}{operation('switch', 'switchDescription')}</div>{!status?.system_changes_enabled && <Text tone="muted">{t('enableChanges')}</Text>}<Card><div className="system-grid">{[['hostLabel', status?.host], ['configuration', status?.config_path], ['currentGeneration', status?.current_system ?? t('currentUnavailable')]].map(([label, value]) => <Stack gap={2} key={label}><Text tone="muted">{t(label!)}</Text><div className="path-value"><div className={label === 'hostLabel' ? 'host-value' : undefined}><Text font={label === 'hostLabel' ? 'heading' : 'mono'} size={label === 'hostLabel' ? 'title' : 'md'}>{value}</Text>{label === 'hostLabel' && (status?.config_kind ?? 'flake') !== 'unknown' && <span className="host-badge"><Badge value={t((status?.config_kind ?? 'flake') === 'legacy' ? 'envLegacyShort' : 'envFlake')} /></span>}</div></div></Stack>)}{(['environment'] as const).map(label => (status?.config_kind ?? 'flake') === 'unknown' ? <Stack gap={2} key={label}><Text tone="muted">{t(label)}</Text><Text tone="muted">{t('missingConfiguration')}</Text></Stack> : null)}</div></Card></>}
 {page === 'history' && <HistoryPage jobs={jobs} selected={selected} log={log} host={selectedHost?.label ?? status?.host ?? ''} language={language} t={t} onSelect={setSelected} onSystem={() => setPage('system')}/>}
 {page === 'maintenance' && <><div className="actions-grid">{operation('update-inputs', 'confirmUpdate')}{operation('rollback', 'confirmRollback')}{operation('clean', 'confirmClean')}</div>{!status?.system_changes_enabled && <Text tone="muted">{t('enableChanges')}</Text>}<Card><Stack gap={3}><Text as="h2" size="title" font="heading" weight="medium">{t('session')}</Text><Button variant="simple" onPress={logout}>{t('logout')}</Button></Stack></Card></>}
 {page === 'connections' && <ConnectionsPage token={token} gateway={gateway ?? ''} t={t} onAuthenticationError={reportError} onChange={async () => { const next = await api<Host[]>(token, 'hosts'); setHosts(next); if (!next.some(item => item.id === host)) switchHost('local'); }}/>}
 {page === 'settings' && <Stack gap={5}><Card><Stack gap={5}>{languageSelect}<Text tone="secondary">{status?.config_path}</Text></Stack></Card>{status?.cache && <Card><Stack gap={4}><Text as="h2" size="title" font="heading">{t('binaryCache')}</Text><Text tone="secondary">{t('cacheDescription')}</Text><div className="path-value"><Text font="mono">{status.cache.url}</Text></div><Text tone="muted">{t('cachePublicKey')}</Text><div className="path-value"><Text font="mono">{status.cache.public_key ?? t('unavailable')}</Text></div></Stack></Card>}</Stack>}
 </div></AppShell></div>;
}
