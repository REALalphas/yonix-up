const files = import.meta.glob('../locales/*.json', { eager: true, import: 'default' }) as Record<string, Record<string, string>>;
export const catalogs = Object.fromEntries(Object.entries(files).map(([path, data]) => [path.split('/').pop()!.replace('.json', ''), data]));
export const names: Record<string, string> = { en: 'English', ru: 'Русский', uk: 'Українська', de: 'Deutsch', es: 'Español', fr: 'Français', it: 'Italiano', ja: '日本語', ko: '한국어', pl: 'Polski', pt: 'Português', tr: 'Türkçe', zh: '简体中文' };
export function resolveLanguage(preference: string, languages: readonly string[]) { if (catalogs[preference])
    return preference; return languages.map(l => l.toLowerCase().split(/[-_]/)[0]).find(l => catalogs[l]) ?? 'en'; }
export function storedLanguage() { try {
    return localStorage.getItem('yonix-language') ?? 'system';
}
catch {
    return 'system';
} }
