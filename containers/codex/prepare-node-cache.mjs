// Build-time only, credential-free. No runtime dependency downloads are allowed.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const hash = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const root = '/opt/workboard-node-profile';
const cache = '/opt/workboard-node-cache/_cacache';
const entries = [];
function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(file);
    else if (entry.isFile()) entries.push([path.relative(cache, file), hash(fs.readFileSync(file))]);
    else throw new Error('cache entry must be regular');
  }
}
walk(cache);
entries.sort((a,b) => a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0);
if (!entries.length) throw new Error('cache must not be empty');
const marker = {
  schema_version: 1, id: fs.existsSync(path.join(root,'scripts/check.js')) ? 'node-npm-v1' : 'node-vite-v1',
  manifest_sha256: hash(fs.readFileSync(path.join(root,'package.json'))),
  lock_sha256: hash(fs.readFileSync(path.join(root,'package-lock.json'))),
  ...(fs.existsSync(path.join(root,'scripts/check.js')) ? {check_sha256: hash(fs.readFileSync(path.join(root,'scripts/check.js')))} : {}),
  cache_files_sha256: hash(JSON.stringify(entries)),
};
fs.writeFileSync(path.join(root,'cache.json'), JSON.stringify(marker)+'\n', {mode:0o444,flag:'wx'});
