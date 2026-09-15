import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const ci = readFileSync(new URL('../../../.github/workflows/ci.yml', import.meta.url), 'utf8');
const release = readFileSync(new URL('../../../.github/workflows/release.yml', import.meta.url), 'utf8');

// Read only the mapping/literal-block forms used by these workflows, failing
// explicitly if their structure changes instead of testing a stale copy.
function block(source, key, indent, optional = false) {
  const lines = source.split(/\r?\n/);
  const declaration = new RegExp(`^ {${indent}}(?:${key}|"${key}"|'${key}'):`);
  const header = new RegExp(`^ {${indent}}${key}:\\s*(?:#.*)?$`);
  const starts = lines.flatMap((line, index) => declaration.test(line) ? [index] : []);
  if (optional && starts.length === 0) return undefined;
  assert.equal(starts.length, 1, `one ${key} block at indentation ${indent}`);
  assert.match(lines[starts[0]], header, `supported multiline ${key} block at indentation ${indent}`);
  const start = starts[0] + 1;
  let end = start;
  while (end < lines.length) {
    const line = lines[end];
    if (line.trim() && line.search(/\S/) <= indent) break;
    end++;
  }
  return lines.slice(start, end).join('\n');
}

const jobs = block(ci, 'jobs', 0);

function permissions(source, indent) {
  const body = block(source, 'permissions', indent, true);
  if (body === undefined) return undefined;
  const result = {};
  for (const line of body.split(/\r?\n/)) {
    if (!line.trim() || line.trimStart().startsWith('#')) continue;
    const entry = line.match(new RegExp(`^ {${indent + 2}}([a-z-]+): (none|read|write)\\s*(?:#.*)?$`));
    assert.ok(entry, `supported explicit permission: ${line}`);
    result[entry[1]] = entry[2];
  }
  return result;
}

function callerCanRunCI(callerSource) {
  const callerJob = block(block(callerSource, 'jobs', 0), 'ci', 2);
  assert.match(callerJob, /^    uses: \.\/\.github\/workflows\/ci\.yml\s*$/m);
  const ceiling = permissions(callerJob, 4) ?? permissions(callerSource, 0) ?? {};
  const requests = [permissions(ci, 0) ?? {}];
  for (const match of jobs.matchAll(/^  ([a-zA-Z0-9_-]+):\s*$/gm)) {
    requests.push(permissions(block(jobs, match[1], 2), 4) ?? {});
  }
  const rank = { none: 0, read: 1, write: 2 };
  return requests.every(request => Object.entries(request).every(([name, level]) =>
    rank[level] <= rank[ceiling[name] ?? 'none']));
}

test('release caller grants every explicit reusable CI permission', () => {
  assert.equal(callerCanRunCI(release), true);
});

test('release caller without PR reads fails the permission contract', () => {
  const fixture = release.replace(/^  pull-requests: read\r?\n/m, '');
  assert.notEqual(fixture, release);
  assert.equal(callerCanRunCI(fixture), false);
});

test('job-level caller permissions replace the inherited ceiling', () => {
  const fixture = release.replace(/^  ci:\r?\n/m, '  ci:\n    permissions:\n      contents: read\n');
  assert.notEqual(fixture, release);
  assert.equal(callerCanRunCI(fixture), false);
});

for (const declaration of [
  'permissions: {}',
  'permissions: { contents: read }',
  'permissions: read-all',
  '"permissions": {}',
]) {
  test(`unsupported job-level declaration fails explicitly: ${declaration}`, () => {
    const fixture = release.replace(/^  ci:\r?\n/m, `  ci:\n    ${declaration}\n`);
    assert.notEqual(fixture, release);
    assert.throws(() => callerCanRunCI(fixture), { code: 'ERR_ASSERTION' });
  });
}
