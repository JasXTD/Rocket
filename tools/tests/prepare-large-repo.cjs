'use strict';
// Synthetic data only. No remote, hooks, credentials, downloaded code or dependencies.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), assert = require('node:assert/strict');
const root = path.resolve('work/mcp-fixtures/large-repo-' + crypto.randomUUID().replaceAll('-', ''));
const repo = path.join(root, 'client-性能'), branch = 'rocket-ui-test/performance';
fs.mkdirSync(repo, { recursive: true }); fs.mkdirSync(path.join(root, 'hooks')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'hooks'), GIT_OPTIONAL_LOCKS: '0' });
const git = (args, input) => cp.execFileSync('git', ['-C', repo, ...args], { env, input, windowsHide: true, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, timeout: 120000 }).trim();
git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Rocket Performance Fixture'], ['user.email', 'fixture@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', path.join(root, 'hooks')], ['gc.auto', '0']]) git(['config', key, value]);
const lines = ['blob\nmark :1\ndata 2\nx\n'];
const count = 20000, files = 5000, unicodeFile = 'src/性能 🚀 é.rs';
const big = Array.from({ length: 2000 }, (_, i) => `let value_${i} = ${i};`).join('\n') + '\n';
for (let i = 1; i <= count; i++) {
  const message = i === 11 ? 'PRIVATE_OLD_NEEDLE_性能' : 'Synthetic step ' + i;
  lines.push(`commit refs/heads/${branch}\nmark :${100 + i}\ncommitter Rocket Fixture <fixture@example.invalid> ${1700000000 + i} +0000\ndata ${Buffer.byteLength(message)}\n${message}\n`);
  if (i > 1) lines.push(`from :${99 + i}\n`);
  else {
    for (let f = 0; f < files; f++) lines.push(`M 100644 :1 files/group-${f % 50}/file-${f}.dat\n`);
    lines.push(`M 100644 inline ${JSON.stringify(unicodeFile)}\ndata ${Buffer.byteLength(big)}\n${big}`);
  }
  const body = `fn step() { let value = ${i}; }\n`;
  lines.push(`M 100644 inline src/perf.rs\ndata ${Buffer.byteLength(body)}\n${body}\n`);
}
for (let i = 0; i < 40; i++) lines.push(`reset refs/heads/rocket-ui-test/branch-${i}\nfrom :${100 + count - i * 300}\n\n`);
lines.push('done\n');
git(['fast-import', '--quiet', '--date-format=raw', '--done'], lines.join(''));
git(['reset', '--hard', 'HEAD']);
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
fs.writeFileSync(path.join(repo, unicodeFile), big.replace(/let value_/g, 'let changed_'));
fs.writeFileSync(path.join(repo, 'src/perf.rs'), 'fn step() { let value = 20001; }\n');
const meta = { root, repo, branch, commits: count, tracked_files: files + 2, unicode_file: unicodeFile, head: git(['rev-parse', 'HEAD']), oldest_match: git(['rev-parse', 'HEAD~19989']), fixture_only: true };
fs.writeFileSync(path.join(root, 'fixture.json'), JSON.stringify(meta, null, 2));
fs.writeFileSync('work/large-repo-fixture-path', root);
console.log(JSON.stringify(meta, null, 2));
