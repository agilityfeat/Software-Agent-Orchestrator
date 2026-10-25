import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';

const vendor = new URL('../demo/vendor/quickjs-0.31.0/', import.meta.url);
test('vendored sandbox dependencies match recorded pinned upstream bytes', async () => {
  const provenance = JSON.parse(await readFile(new URL('provenance.json', vendor), 'utf8'));
  assert.equal(provenance.version, '0.31.0');
  assert.equal(provenance.files.length, 6);
  for (const file of provenance.files) {
    assert.match(file.source, /@0\.31\.0\//);
    const contents = await readFile(new URL(file.file, vendor));
    assert.equal(createHash('sha256').update(contents).digest('hex'), file.sha256, file.file);
  }
});

test('QuickJS sandbox keeps its module dependency graph on the same origin', async () => {
  const queue = [new URL('../demo/sandbox-worker.mjs', import.meta.url)];
  const seen = new Set();
  while (queue.length) {
    const file = queue.shift();
    if (seen.has(file.href)) continue;
    seen.add(file.href);
    const source = await readFile(file, 'utf8');
    for (const match of source.matchAll(/(?:from\s*|import\s*\()(['"])([^'"]+)\1/g)) {
      assert.ok(match[2].startsWith('./'), `Nonlocal sandbox dependency in ${file.pathname}: ${match[2]}`);
      queue.push(new URL(match[2], file));
    }
  }
  assert.equal(seen.size, 5); // worker, core, variant, FFI, embedded WASM module
  const html = await readFile(new URL('../demo/index.html', import.meta.url), 'utf8');
  assert.match(html, /worker-src 'self' blob:;/);
  assert.doesNotMatch(html, /(?:^|[ ;'])unsafe-eval(?:'|[ ;])/);
});
