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
const scope = block(jobs, 'windows_scope', 2);
assert.equal((scope.match(/^        id: scope\s*$/gm) ?? []).length, 1);
assert.equal((scope.match(/uses: actions\/github-script@/g) ?? []).length, 1);
const scriptLines = block(scope, 'with', 8).split(/\r?\n/);
assert.equal(scriptLines[0], '          script: |');
for (const line of scriptLines.slice(1)) {
  assert.ok(!line.trim() || line.startsWith('            '), 'classifier literal indentation');
}
const source = scriptLines.slice(1).map(line => line.slice(12)).join('\n');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const classify = new AsyncFunction('context', 'github', 'core', source);

async function runClassifier(files, eventName = 'pull_request', apiFailure) {
  const outputs = {};
  const endpoint = Symbol('pulls.listFiles');
  let apiCalls = 0;
  const context = { eventName, repo: { owner: 'mirurobotics', repo: 'agent' } };
  if (eventName === 'pull_request') context.payload = { pull_request: { number: 236 } };
  const github = {
    rest: { pulls: { listFiles: endpoint } },
    paginate: async (actualEndpoint, parameters) => {
      apiCalls++;
      assert.equal(actualEndpoint, endpoint);
      assert.deepEqual(parameters, { owner: 'mirurobotics', repo: 'agent', pull_number: 236, per_page: 100 });
      if (apiFailure) throw apiFailure;
      return files;
    },
  };
  const promise = classify(context, github, { setOutput: (key, value) => { outputs[key] = value; } });
  if (apiFailure) {
    await assert.rejects(promise, error => error === apiFailure);
    assert.deepEqual(outputs, {});
  } else {
    await promise;
  }
  assert.equal(apiCalls, eventName === 'pull_request' ? 1 : 0);
  return outputs;
}

const packagePaths = ['build/windows/miru-agent.wxs', '.github/workflows/ci.yml', '.github/workflows/release.yml'];
const compilePaths = ['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', 'agent/src/main.rs', 'libs/device-api/src/lib.rs'];
for (const [paths, windows_package] of [[packagePaths, true], [compilePaths, false]]) {
  for (const path of paths) {
    for (const [change, file] of [
      ['ordinary', { filename: path }],
      ['renamed out', { filename: 'archive/input.txt', previous_filename: path }],
      ['renamed in', { filename: path, previous_filename: 'archive/input.txt' }],
    ]) {
      test(`${change}: ${path}`, async () => {
        assert.deepEqual(await runClassifier([file]), { windows_compile: true, windows_package });
      });
    }
  }
}

for (const [name, files] of [
  ['empty', []],
  ['documentation', [{ filename: 'README.md' }]],
  ['irrelevant rename', [{ filename: 'docs/new.md', previous_filename: 'docs/old.md' }]],
  ['absent prior names', [{ filename: 'docs/a.md', previous_filename: null }, { filename: 'docs/b.md', previous_filename: '' }]],
]) {
  test(name, async () => {
    assert.deepEqual(await runClassifier(files), { windows_compile: false, windows_package: false });
  });
}

test('mixed inputs retain the strongest Windows validation', async () => {
  const files = ['README.md', 'agent/src/main.rs', 'build/windows/README.md'].map(filename => ({ filename }));
  assert.deepEqual(await runClassifier(files), { windows_compile: true, windows_package: true });
});

for (const event of ['push', 'workflow_call']) {
  test(`${event} validates packages without PR data or an API call`, async () => {
    assert.deepEqual(await runClassifier(undefined, event), { windows_compile: true, windows_package: true });
  });
}

test('pagination failure propagates without successful scope outputs', async () => {
  await runClassifier([], 'pull_request', new Error('injected API failure'));
});

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
