export type Action = 'check' | 'build' | 'diff' | 'switch' | 'update-inputs' | 'rollback' | 'clean';
export interface Job {
    id: string;
    action: Action;
    status: string;
    started: number;
    finished?: number | null;
    error?: string;
}
export type ConfigKind = 'flake' | 'legacy' | 'unknown';
export interface Status {
    host: string;
    config_path: string;
    current_system: string | null;
    active: string | null;
    system_changes_enabled: boolean;
    /** Environment type. Older (flake-only) peers omit it; treat as flake. */
    config_kind?: ConfigKind;
    cache?: {url:string;public_key:string|null}|null;
}
export const actionKeys: Record<Action, string> = { check: 'actionCheck', build: 'actionBuild', diff: 'actionDiff', switch: 'actionSwitch', 'update-inputs': 'actionUpdate', rollback: 'actionRollback', clean: 'actionClean' };
export const confirmations: Partial<Record<Action, string>> = { switch: 'confirmSwitch', 'update-inputs': 'confirmUpdate', rollback: 'confirmRollback', clean: 'confirmClean' };
export function allowed(action: Action, status: Status | null, pending: boolean) { return !!status && !status.active && !pending && (status.system_changes_enabled || !['switch', 'rollback', 'clean'].includes(action)) && (action !== 'update-inputs' || (status.config_kind ?? 'flake') === 'flake'); }
export class ApiError extends Error {
    constructor(public readonly status: number, message: string) {
        super(message);
        this.name = 'ApiError';
    }
}
export async function api<T>(token: string, path: string, signal?: AbortSignal, body?: unknown): Promise<T> {
    const response = await fetch('/api/' + path, { signal, method: body ? 'POST' : 'GET', headers: { Authorization: 'Bearer ' + token, ...(body ? { 'Content-Type': 'application/json' } : {}) }, ...(body ? { body: JSON.stringify(body) } : {}) });
    if (!response.ok) {
        let message = String(response.status);
        try {
            message = (await response.json()).error ?? message;
        }
        catch { }
        throw new ApiError(response.status, message);
    }
    return path.endsWith('/log') ? await response.text() as T : response.json();
}
export interface Host { id: string; label: string; local: boolean }
export function hostPath(host: string, path: string) { return 'hosts/' + encodeURIComponent(host) + '/' + path; }
export interface Connection { id: string; label: string; url: string; managed_by_nix: boolean }
export interface ConnectionCheck { host: string; version: string }
const tokenStorageKey = 'yonix-access-token';
export function storedToken() {
    try {
        const token = localStorage.getItem(tokenStorageKey) ?? '';
        return /^[a-f\d]{64}$/i.test(token) ? token : '';
    }
    catch { return ''; }
}
export function saveToken(token: string) {
    try { localStorage.setItem(tokenStorageKey, token); return true; }
    catch { return false; }
}
export function clearToken() {
    try { localStorage.removeItem(tokenStorageKey); }
    catch { }
}
