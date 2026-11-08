import { useEffect, useMemo, useRef } from 'react';
import { Button, Card, DrawerButton, Stack, Text } from '@alpha/react';
import { actionKeys, type Job } from './model';

type Props = {
    jobs: Job[];
    selected: string | null;
    log: string;
    host: string;
    language: string;
    t: (key: string) => string;
    onSelect: (id: string) => void;
    onSystem: () => void;
};

export function HistoryPage({ jobs, selected, log, host, language, t, onSelect, onSystem }: Props) {
    const ordered = useMemo(() => [...jobs].reverse().sort((a, b) => a.started - b.started), [jobs]);
    const current = jobs.find(job => job.id === selected);
    const strip = useRef<HTMLDivElement>(null);
    const activeItem = useRef<HTMLDivElement>(null);
    const newest = ordered.at(-1)?.id;
    useEffect(() => {
        const viewport = strip.current, item = activeItem.current;
        if (!viewport) return;
        if (item) {
            const bounds = item.getBoundingClientRect(), frame = viewport.getBoundingClientRect();
            if (bounds.left < frame.left) viewport.scrollLeft += bounds.left - frame.left;
            else if (bounds.right > frame.right) viewport.scrollLeft += bounds.right - frame.right;
        } else viewport.scrollLeft = viewport.scrollWidth;
    }, [selected, newest]);
    if (!jobs.length) return <Card><Stack gap={4}><Text>{t('noOperations')}</Text><Button onPress={onSystem}>{t('system')}</Button></Stack></Card>;
    return <>
        <div className="history-strip" ref={strip} role="region" aria-label={t('history')} tabIndex={0}>
            {ordered.map(job => <div className="history-item" key={job.id} ref={job.id === selected ? activeItem : undefined}>
                <DrawerButton label={t(actionKeys[job.action])} icon={job.status === 'succeeded' ? 'check' : job.status === 'running' ? 'refresh' : 'info'} selected={job.id === selected} onPress={() => onSelect(job.id)}/>
            </div>)}
        </div>
        {current && <Card><Stack gap={4}>
            <Text as="h2" font="heading" size="title" weight="medium">{t(actionKeys[current.action])}</Text>
            <div className="operation-details">
                {[[t('hostLabel'), host], [t('operationStatus'), t(current.status)], [t('operationStarted'), new Date(current.started * 1000).toLocaleString(language)], ...(current.finished ? [[t('operationFinished'), new Date(current.finished * 1000).toLocaleString(language)]] : []), [t('operationId'), current.id]].map(([label, value]) => <Stack gap={2} key={label}><Text tone="muted">{label}</Text><div className="path-value"><Text>{value}</Text></div></Stack>)}
            </div>
            {current.error && <div role="alert"><Text>{current.error}</Text></div>}
        </Stack></Card>}
        <Card><Stack gap={4}>
            <Text as="h2" size="title" font="heading" weight="medium">{t('operationLog')}</Text>
            <div className="log-output" role="region" aria-label={t('operationLog')} tabIndex={0}><Text font="mono">{selected ? (log || t('loadingLog')) : t('logHint')}</Text></div>
        </Stack></Card>
    </>;
}
