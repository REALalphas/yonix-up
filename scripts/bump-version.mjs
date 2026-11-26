#!/usr/bin/env node
import { readFile, writeFile } from 'node:fs/promises';

const component = process.argv[2] ?? 'patch';
if (!['patch', 'minor'].includes(component)) {
    console.error('Usage: node scripts/bump-version.mjs [patch|minor]');
    process.exit(2);
}

const semver = /^(\d+)\.(\d+)\.(\d+)$/;
const cargoPath = 'Cargo.toml';
const cargo = await readFile(cargoPath, 'utf8');
const match = cargo.match(/^version = "([^"]+)"$/m);
if (!match || !semver.test(match[1])) {
    throw new Error(`${cargoPath} does not contain a valid package version`);
}

const [, majorText, minorText, patchText] = match[1].match(semver);
const major = Number(majorText);
const minor = Number(minorText) + (component === 'minor' ? 1 : 0);
const patch = component === 'minor' ? 0 : Number(patchText) + 1;
const next = `${major}.${minor}.${patch}`;

function replaceVersion(text, path, pattern, current) {
    const updated = text.replace(pattern, (_, prefix, old, suffix) => {
        if (old !== current) throw new Error(`${path} is out of sync with ${cargoPath}`);
        return `${prefix}${next}${suffix}`;
    });
    if (updated === text) throw new Error(`${path} does not contain the expected Yonix version`);
    return updated;
}

const lockPath = 'Cargo.lock';
const lock = await readFile(lockPath, 'utf8');
const packagePath = 'package.json';
const packageJson = JSON.parse(await readFile(packagePath, 'utf8'));
if (packageJson.version !== match[1]) throw new Error(`${packagePath} is out of sync with ${cargoPath}`);
const packageLockPath = 'package-lock.json';
const packageLock = JSON.parse(await readFile(packageLockPath, 'utf8'));
if (packageLock.version !== match[1] || packageLock.packages?.['']?.version !== match[1]) {
    throw new Error(`${packageLockPath} is out of sync with ${cargoPath}`);
}
const nixPaths = ['nix/package.nix', 'nix/web.nix'];
const nixSources = await Promise.all(nixPaths.map(path => readFile(path, 'utf8')));
for (const [index, text] of nixSources.entries()) {
    if (/version = "[^"]+"/.test(text)) throw new Error(`${nixPaths[index]} still contains a duplicated version literal`);
}
const changelogPath = 'CHANGELOG.md';
const changelog = await readFile(changelogPath, 'utf8');
const updatedChangelog = changelog.replace(
    /^## Unreleased — \d+\.\d+\.\d+ development$/m,
    `## Unreleased — ${next} development`,
);
if (updatedChangelog === changelog) throw new Error(`${changelogPath} does not contain the unreleased version heading`);

packageJson.version = next;
packageLock.version = next;
packageLock.packages[''].version = next;
const updatedCargo = replaceVersion(cargo, cargoPath, /^(version = ")([^"]+)(")$/m, match[1]);
const updatedLock = replaceVersion(lock, lockPath, /(name = "yonix"\nversion = ")([^"]+)(")/, match[1]);
await Promise.all([
    writeFile(cargoPath, updatedCargo),
    writeFile(lockPath, updatedLock),
    writeFile(packagePath, `${JSON.stringify(packageJson, null, 2)}\n`),
    writeFile(packageLockPath, `${JSON.stringify(packageLock, null, 2)}\n`),
    writeFile(changelogPath, updatedChangelog),
]);

console.log(`${match[1]} -> ${next}`);
