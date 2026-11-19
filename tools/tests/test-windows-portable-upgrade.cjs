// Run two reviewed Windows releases through the native updater in a disposable installation.
'use strict';
const assert = require('node:assert/strict');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const net = require('node:net');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const fixture = require('./app-update-fixtures.cjs');

const args = process.argv.slice(2);
const expectProgress = args.includes('--expect-progress');
const [oldZip, oldHash, newZip, newHash, output, interruptedReport] = args.filter(arg => arg !== '--expect-progress');
assert(process.platform === 'win32' && output && [oldHash, newHash].every(h => /^[a-f0-9]{64}$/.test(h)),
  'Usage: node test-windows-portable-upgrade.cjs OLD_ZIP OLD_SHA256 NEW_ZIP NEW_SHA256 NEW_REPORT [INTERRUPTED_REPORT] [--expect-progress]');
assert(!fs.existsSync(output), 'Choose a new report path');
const parent = path.resolve('work/mcp-fixtures');
fs.mkdirSync(parent, {recursive: true});
const prior = interruptedReport && JSON.parse(fs.readFileSync(interruptedReport, 'utf8'));
assert(!(expectProgress && prior && !prior.progress), 'Visible progress must be observed during the install');
const root = prior ? fs.realpathSync(prior.fixture) : fs.mkdtempSync(path.join(fs.realpathSync(parent), 'portable-upgrade-'));
function checkFixture() {
  assert.equal(path.dirname(fs.realpathSync(root)), fs.realpathSync(parent));
  assert.match(path.basename(root), /^portable-upgrade-[A-Za-z0-9]{6}$/);
  assert.equal(fs.readFileSync(path.join(root, 'rocket-private-state'), 'utf8').trim(), 'synthetic-content-only-v1');
}
if (prior) {
  checkFixture();
  assert.equal(root, prior.fixture);
  assert.equal(prior.from.sha256, oldHash);
  assert.equal(prior.to.sha256, newHash);
} else fs.writeFileSync(path.join(root, 'rocket-private-state'), 'synthetic-content-only-v1\n', {flag: 'wx'});
const target = path.join(root, 'installation'), repo = path.join(root, 'repository');
const cache = path.join(root, 'rocket-ui-state', 'updates');
const desktop = path.join(target, 'rocket.exe');
// Full release preparation copies and rehashes thousands of files before handoff.
const packageTimeout = 20 * 60 * 1000;
const env = {...process.env};
for (const key of Object.keys(env)) {
  if (/^(GIT_|GCM_|SSH_|ROCKET_|GH_|GITHUB_|CURL_|SSL_CERT_)|_PROXY$/i.test(key)) delete env[key];
}
Object.assign(env, {
  LOCALAPPDATA: path.join(root, 'local'), APPDATA: path.join(root, 'roaming'),
  USERPROFILE: path.join(root, 'user'), TEMP: path.join(root, 'temp'), TMP: path.join(root, 'temp'),
  GIT_CONFIG_GLOBAL: path.join(root, 'gitconfig'), GIT_CONFIG_NOSYSTEM: '1',
  GIT_TEMPLATE_DIR: path.join(root, 'templates'), GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0',
});
for (const dir of [env.LOCALAPPDATA, env.APPDATA, env.USERPROFILE, env.TEMP, env.GIT_TEMPLATE_DIR, repo, cache])
  fs.mkdirSync(dir, {recursive: true});
if (!prior) fs.writeFileSync(env.GIT_CONFIG_GLOBAL, '', {flag: 'wx'});
const report = {passed: false, checks: []};
const pass = message => { report.checks.push(message); console.log('PASS ' + message); };
function run(program, args, extra = {}) {
  return cp.execFileSync(program, args, {cwd: root, env, windowsHide: true, encoding: 'utf8',
    maxBuffer: 8 * 1024 * 1024, timeout: 300000, ...extra});
}
const shell = path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe');
const ps = (script, extra) => run(shell, ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', script], extra);
const extract = fs.readFileSync(path.resolve('crates/rocket-git/src/app_update/extract.ps1'), 'utf8');
async function readRelease(filename, expectedHash) {
  const file = fs.realpathSync(filename);
  const version = /^Rocket-(\d+\.\d+\.\d+)-windows-x64\.zip$/.exec(path.basename(file))?.[1];
  assert(version, 'Expected a Windows portable ZIP');
  const digest = crypto.createHash('sha256');
  for await (const bytes of fs.createReadStream(file)) digest.update(bytes);
  assert.equal(digest.digest('hex'), expectedHash, 'Archive must match its reviewed digest');
  const sums = ps([
    "$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)",
    'Add-Type -AssemblyName System.IO.Compression.FileSystem',
    '$zip=[IO.Compression.ZipFile]::OpenRead($env:ROCKET_TEST_ARCHIVE)',
    "try { $s=[IO.StreamReader]::new($zip.GetEntry('SHA256SUMS.txt').Open()); try { [Console]::Write($s.ReadToEnd()) } finally { $s.Dispose() } } finally { $zip.Dispose() }",
  ].join('\n'), {env: {...env, ROCKET_TEST_ARCHIVE: file}});
  const files = new Map();
  for (const line of sums.trimEnd().split(/\r?\n/)) {
    const match = /^([a-f0-9]{64})  (.+)$/.exec(line);
    assert(match && !files.has(match[2]), 'Invalid package inventory');
    files.set(match[2], match[1]);
  }
  files.set('SHA256SUMS.txt', fixture.sha(Buffer.from(sums)));
  return {file, version, files, size: fs.statSync(file).size, hash: expectedHash};
}
function verifyTree(directory, release) {
  const actual = fixture.inventory(directory);
  assert.equal(Object.keys(actual).length, release.files.size);
  for (const [name, hash] of release.files) assert.equal(actual[name], hash, 'Package mismatch: ' + name);
}
const git = (...args) => run('git.exe', ['-C', repo, ...args]);
function repoState() {
  return [git('rev-parse', 'HEAD'), git('ls-files', '--stage'), git('status', '--porcelain'),
    fs.readFileSync(path.join(repo, 'tracked'), 'utf8'), fs.readFileSync(path.join(repo, 'untracked'), 'utf8')];
}
async function until(test, label, ms = 240000) {
  const end = Date.now() + ms;
  while (Date.now() < end) { const value = await test(); if (value) return value; await delay(150); }
  throw Error('Timed out waiting for ' + label);
}
let server, service, client, state, profile, asset;
const incomplete = fixture.zip({'ordinary.md': Buffer.from('A document without release metadata.')});
const invalid = {size: incomplete.length, hash: fixture.sha(incomplete)};
function offer(version, body) {
  const metadata = fixture.release(version, Buffer.alloc(0));
  metadata.assets[0].size = body.size;
  metadata.assets[0].digest = 'sha256:' + body.hash;
  asset = {metadata, body};
}
async function serve() {
  server = http.createServer((req, res) => {
    if (req.url === '/repos/JasXTD/Rocket/releases/latest') res.end(JSON.stringify(asset.metadata));
    else if (req.url === '/repos/JasXTD/Rocket/releases/assets/7') {
      res.writeHead(200, {'Content-Type': 'application/octet-stream', 'Content-Length': asset.body.size});
      if (asset.body.file) {
        const stream = fs.createReadStream(asset.body.file);
        res.on('close', () => stream.destroy());
        stream.pipe(res);
      } else res.end(incomplete);
    } else { res.writeHead(404); res.end(); }
  });
  // A normal desktop checks for public updates on startup. Its inherited proxy
  // terminates that attempt locally. The fixture updater uses loopback directly.
  server.on('connect', (_request, socket) => socket.end('HTTP/1.1 403 Forbidden\r\n\r\n'));
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = 'http://127.0.0.1:' + server.address().port;
  Object.assign(env, {HTTP_PROXY: url, HTTPS_PROXY: url, ALL_PROXY: url, NO_PROXY: '127.0.0.1,localhost'});
  fs.writeFileSync(path.join(root, 'rocket-update-api'), url);
}
async function waitState(predicate, label, failOnError = false, timeout) {
  return until(async () => {
    state = await client.tool('rocket_state');
    if (failOnError && state.updates?.status === 'error') throw Error(label + ': ' + state.updates.error);
    return predicate(state);
  }, label, timeout);
}
async function click(id) {
  await until(async () => {
    state = await client.tool('rocket_state');
    if (state.input_pending || !state.controls?.some(c => c.id === id && c.enabled)) return false;
    if (id !== 'update-stop' && (state.busy || state.tabs?.some(t => t.loading))) return false;
    try { await client.tool('rocket_click', {target: id, expected_tab: state.active_tab}); }
    catch (e) {
      // The native host can begin a repository refresh between our read and click.
      if (e.message === 'Wait for the current repository operation to settle') return false;
      throw e;
    }
    return true;
  }, id);
}
async function startHost(version) {
  profile = path.join(root, 'profile-' + version + '-' + crypto.randomUUID());
  const exe = path.join(target, 'rocket-mcp.exe');
  run(exe, ['--init-http', profile, root]);
  const portProbe = net.createServer();
  await new Promise(resolve => portProbe.listen(0, '127.0.0.1', resolve));
  const port = portProbe.address().port;
  await new Promise(resolve => portProbe.close(resolve));
  fs.writeFileSync(path.join(profile, 'settings'), ['ROCKET-MCP-1', port, root, ''].join('\n'));
  const log = fs.openSync(path.join(root, version + '-service.log'), 'w');
  try { service = cp.spawn(exe, ['--http-profile', profile], {env, cwd: root, windowsHide: true,
    stdio: ['ignore', 'ignore', log]}); } finally { fs.closeSync(log); }
  client = new RocketHttpClient(profile);
  await until(async () => {
    assert.equal(service.exitCode, null, 'Packaged host exited unexpectedly');
    try { return (await client.raw('GET', '/health')).status === 200; } catch { return false; }
  }, 'packaged host');
  assert.equal((await client.initialize()).serverInfo.version, version);
  await client.tool('rocket_open', {path: repo});
  await waitState(s => s.updates?.status === 'available' && !s.updates.running && !s.busy &&
    s.tabs?.length && !s.tabs.some(tab => tab.loading), 'available update and loaded repository', true);
}
async function shutdown(handedOff = false) {
  if (service?.exitCode === null && client) {
    const closed = async () => {
      const health = await client.raw('GET', '/health');
      assert.equal(health.status, 200);
      const lines = health.text.trim().split(/\r?\n/);
      assert.equal(Number(lines[2]), service.pid);
      return ['0','3'].includes(lines[3]);
    };
    if (!handedOff && !(await closed())) {
      // The old frame can disappear before a close call receives its response.
      // /health is served by the host even when there is no window to answer RPCs.
      try { await client.tool('rocket_close'); }
      catch (e) { if (!e.message.startsWith('MCP HTTP 504:')) throw e; }
      let confirmed = false;
      await until(async () => {
        if (await closed()) return true;
        try {
          const view = await client.tool('rocket_state');
          if (view.dialog?.kind === 'quit' && !confirmed) {
            confirmed = true;
            await client.tool('rocket_click', {target: 'dialog-submit', expected_tab: view.active_tab});
          }
        } catch (e) { if (!e.message.startsWith('MCP HTTP 504:')) throw e; }
        return false;
      }, 'test window close');
    }
    await until(async () => {
      const response = await client.raw('POST', '/shutdown');
      if (response.status === 409) return false;
      assert.equal(response.status, 202, 'Packaged host rejected shutdown');
      return true;
    }, 'old window to release its host', 30000);
    await until(() => service.exitCode !== null, 'host exit', 30000);
    assert.equal(service.exitCode, 0);
  }
  if (client) await client.finish();
  if (profile) fs.rmSync(path.join(profile, 'access-token'), {force: true});
  client = service = undefined;
}
async function review() {
  if (state.updates.notice_visible) await click('update-notice-later');
  if (state.updates.open) await click('update-later');
  await click('menu-help'); await click('help-updates');
  await waitState(s => s.updates.open && s.updates.status === 'available' && !s.updates.running &&
    s.updates.release?.size === asset.body.size, 'release review');
}
const cacheNames = () => fs.readdirSync(cache).sort();
const approvedJob = () => cacheNames().map(n => path.join(cache,n)).find(p => fs.existsSync(path.join(p,'approved.json')));
const progressPhases = ['check_installed', 'check_update', 'stage'];
function recordProgress(samples, old, next) {
  for (const phase of progressPhases) {
    const s = samples[phase];
    assert(s?.visible && s.frames >= 2 && s.first >= 0 && s.first < s.last && s.last <= s.total,
      'Missing advancing, visible file count for ' + phase);
    assert.equal(s.total, (phase === 'check_installed' ? old : next).files.size);
  }
  report.progress = samples;
  pass('The native restart dialog showed advancing, bounded file counts for all three phases');
}
async function watchRestart(old, next) {
  const samples = {};
  let lastPhase = -1;
  await until(async () => {
    if (approvedJob()) return true;
    let view;
    try { view = await client.tool('rocket_state'); }
    catch (e) {
      // Only the read can race the closing window. Install was already submitted.
      if (e.message.startsWith('MCP HTTP 504:')) return false;
      throw e;
    }
    if (view.updates?.status === 'error') throw Error(view.updates.error);
    const p = view.updates?.file_progress;
    if (!p) return false;
    const index = progressPhases.indexOf(p.phase);
    assert(index >= 0 && index >= lastPhase, 'Unknown or regressing restart phase');
    if (index !== lastPhase) console.log('Restart phase ' + p.phase + ' covers ' + p.total + ' files');
    lastPhase = index;
    assert.equal(p.total, (index === 0 ? old : next).files.size);
    assert(Number.isInteger(p.done) && p.done >= 0 && p.done <= p.total);
    const sample = samples[p.phase] ??= {first: p.done, last: p.done, total: p.total, frames: 0};
    assert(p.done >= sample.last, 'Verified file count went backwards');
    sample.last = p.done;
    sample.frames++;
    const bar = view.controls.find(c => c.id === 'update-file-progress');
    if (p.done > 0 && p.done < p.total && bar && !sample.visible) {
      const dialog = view.controls.find(c => c.id === 'update-dialog');
      const [x, y, right, bottom] = bar.rect;
      assert(dialog && right > x && bottom > y && x >= dialog.rect[0] && right <= dialog.rect[2] &&
        y >= dialog.rect[1] && bottom <= dialog.rect[3], 'Progress bar outside the dialog');
      sample.visible = true;
      if (p.phase === 'check_update') {
        await client.tool('rocket_resize', {width: 780, height: 620});
      }
    }
    if (sample.visible && p.phase !== 'check_update' && !sample.screenshot && p.done >= p.total / 3 && p.done < p.total) {
      const shot = await client.request('tools/call', {name: 'rocket_capture', arguments: {}});
      const png = shot.content.find(c => c.type === 'image');
      assert(!shot.isError && png, 'Missing native progress screenshot');
      const imagePath = output.replace(/\.json$/i, '') + '-' + p.phase + '.png';
      fs.writeFileSync(imagePath, Buffer.from(png.data, 'base64'), {flag:'wx'});
      sample.screenshot = path.basename(imagePath);
    }
    return false;
  }, 'file progress and approved restart', packageTimeout);
  recordProgress(samples, old, next);
}
const retained = path.join(cache, 'prior-attempt');
const baseline = path.join(root, 'repository-before.json');
const desktopFilter = "$p=@(Get-Process -Name rocket -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:ROCKET_TEST_DESKTOP });";
function inspectDesktop(script) {
  return ps("$ErrorActionPreference='Stop';" + desktopFilter + script + '; exit 0',
    {env: {...env, ROCKET_TEST_DESKTOP: desktop}}).trim();
}
async function closeDesktop() {
  await until(() => inspectDesktop('if(!$p.Count -or ($p.Count -eq 1 -and $p[0].CloseMainWindow())){[Console]::Write(1)}'),
    'disposable desktop to accept close');
  await until(() => !inspectDesktop('if($p.Count){[Console]::Write($p.Count)}'), 'disposable desktop exit');
}
async function install(old, next) {
  console.log('Preparing a disposable copy of Rocket ' + old.version);
  ps(extract, {input: old.file + '\n' + target + '\n'});
  verifyTree(target, old);
  git('init', '-b', 'rocket-ui-test/portable-upgrade');
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), 'rocket-ui-test/portable-upgrade');
  fs.writeFileSync(path.join(repo, 'tracked'), 'original\n'); git('add', 'tracked');
  git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-m', 'fixture');
  fs.appendFileSync(path.join(repo, 'tracked'), 'uncommitted edit\n');
  fs.writeFileSync(path.join(repo, 'untracked'), 'preserve\n');
  fs.writeFileSync(baseline, JSON.stringify(repoState()), {flag: 'wx'});
  fs.mkdirSync(retained); fs.writeFileSync(path.join(retained, 'marker'), 'keep');
  offer(next.version, invalid);
  await startHost(old.version); await review(); await click('update-download');
  await waitState(s => s.updates.status === 'error' && !s.updates.running, 'old missing-metadata result');
  console.log('Downloading the real ' + next.version + ' ZIP through the old updater');
  offer(next.version, next); await review(); await click('update-download');
  await waitState(s => s.updates.ready && s.updates.installable, 'verified real release', true, packageTimeout);
  assert.equal(state.updates.release.version, next.version);
  console.log('Staging the verified ZIP in the disposable installation');
  await click('update-install');
  // Approval outlives the closing UI. Never replay an install to find out whether it succeeded.
  if (expectProgress) await watchRestart(old, next);
  else await until(approvedJob, 'approved restart', packageTimeout);
  await shutdown(true);
}
function recordedEvents(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, 'utf8').split('\n').slice(0, -1).map(line => JSON.parse(line));
}
async function test() {
  const old = await readRelease(oldZip, oldHash), next = await readRelease(newZip, newHash);
  report.from = {version: old.version, sha256: old.hash}; report.to = {version: next.version, sha256: next.hash};
  await serve();
  if (!prior) await install(old, next);
  const job = approvedJob();
  assert(job && JSON.parse(fs.readFileSync(path.join(job, 'approved.json'))).install === true, 'Missing approval receipt');
  assert.equal(fs.readFileSync(path.join(retained,'marker'),'utf8'), 'keep');
  const leftover = cacheNames().filter(n => n.startsWith('update-') && path.join(cache,n) !== job);
  report.oldFailedJobs = leftover.length;
  report.oldRetainedArchiveAndFile = leftover.some(n =>
    fs.existsSync(path.join(cache,n,'release.zip')) && fs.existsSync(path.join(cache,n,'bundle','ordinary.md')));
  if (prior) {
    assert.equal(report.oldRetainedArchiveAndFile, prior.oldRetainedArchiveAndFile);
    const completedOldRun = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).some(n => {
      const rows = recordedEvents(path.join(root, n, 'rocket.jsonl'));
      const session = rows.find(r => r.event === 'session_start' && r.version === old.version);
      return session && ['app-update-download', 'app-update-install'].every(op =>
        rows.some(r => r.event === 'request_end' && r.pid === session.pid && r.operation === op && r.outcome === 'ok'));
    });
    assert(completedOldRun, 'Missing completed old-version updater operations');
    report.resumedAfterApprovedInstall = true;
    if (expectProgress) recordProgress(prior.progress, old, next);
  }
  const before = repoState();
  if (fs.existsSync(baseline)) assert.deepEqual(before, JSON.parse(fs.readFileSync(baseline)));
  else {
    assert(prior, 'Missing original repository state');
    assert.equal(git('branch', '--show-current').trim(), 'rocket-ui-test/portable-upgrade');
    assert.equal(git('rev-list', '--count', '--all').trim(), '1');
    assert.equal(git('show', 'HEAD:tracked'), 'original\n');
    assert.equal(before[2].replaceAll('\r\n','\n'), ' M tracked\n?? untracked\n');
    assert.deepEqual(before.slice(3), ['original\nuncommitted edit\n', 'preserve\n']);
  }
  const result = path.join(job, 'result.json');
  const receipt = await until(() => {
    // The helper creates, writes and syncs the receipt before launching the new desktop.
    try { return JSON.parse(fs.readFileSync(result, 'utf8')); }
    catch (e) { if (e.code === 'ENOENT' || e instanceof SyntaxError) return false; throw e; }
  }, 'complete update receipt', packageTimeout);
  assert.equal(receipt.code, 'installed', receipt.message); assert.equal(receipt.version, next.version);
  const plan = JSON.parse(fs.readFileSync(path.join(job, 'install.json')));
  const logDir = fs.readdirSync(root).find(n => n.startsWith('mcp-logs-' + plan.parent + '-'));
  const events = recordedEvents(path.join(root, logDir, 'rocket.jsonl'));
  const duration = op => {
    const row = events.find(r => r.event === 'request_end' && r.operation === op && r.outcome === 'ok');
    assert(row && Number.isFinite(row.elapsed_ms), 'Missing successful ' + op + ' duration');
    return row.elapsed_ms;
  };
  report.timings = {
    downloadAndVerifyMs: duration('app-update-download'),
    prepareRestartMs: duration('app-update-install'),
    approvalToInstallReceiptMs: Math.round(fs.statSync(result).birthtimeMs - fs.statSync(path.join(job, 'approved.json')).birthtimeMs),
  };
  // The initial incomplete-archive case is earlier in the same log.
  for (const [step, field] of [['download_bundle', 'transferMs'], ['extract_bundle', 'extractMs']]) {
    const row = events.findLast(r => r.event === 'step_end' && r.step === step && r.outcome === 'ok');
    if (row) report.timings[field] = row.elapsed_ms;
  }
  console.log('Preparing to restart took ' + (report.timings.prepareRestartMs / 1000).toFixed(1) + ' s');
  const log = path.join(env.LOCALAPPDATA, 'Rocket', 'logs', 'rocket.jsonl');
  const installedAt = fs.statSync(result).birthtimeMs;
  const desktopSession = await until(() => {
    const rows = recordedEvents(log);
    const session = rows.find(r => r.event === 'session_start' && r.version === next.version && r.time_unix_ms >= Math.floor(installedAt));
    const runtime = session && rows.find(r => r.session === session.session && r.event === 'git_version');
    return runtime && {pid: session.pid, git: [runtime.major, runtime.minor, runtime.patch].join('.')};
  }, 'restarted desktop Git initialization', packageTimeout);
  report.desktopPid = desktopSession.pid;
  report.desktopGit = desktopSession.git;
  const liveWindow = () => inspectDesktop('if($p.Count -eq 1 -and $p[0].MainWindowHandle -ne 0){[Console]::Write($p[0].Id)}');
  const pid = prior ? liveWindow() : await until(liveWindow, 'restarted desktop window');
  if (pid) assert.equal(Number(pid), report.desktopPid);
  report.observedLiveWindow = Boolean(pid) || Boolean(prior?.observedLiveWindow && prior.desktopPid === desktopSession.pid);
  assert.equal(JSON.parse(run(path.join(target, 'rocket-cli.exe'), ['--json', '--version'])).data.version, next.version);
  const gitRoot = path.join(target, 'git');
  const runtimes = fs.readdirSync(gitRoot).filter(n => fs.existsSync(path.join(gitRoot,n,'cmd/git.exe')));
  assert.equal(runtimes.length, 1);
  report.bundledGit = run(path.join(gitRoot,runtimes[0],'cmd/git.exe'), ['--version']).trim();
  assert.equal(report.bundledGit, 'git version ' + runtimes[0]);
  assert.equal(report.desktopGit, runtimes[0].split('.').slice(0, 3).join('.'));
  assert.equal(path.toNamespacedPath(fs.realpathSync(path.dirname(receipt.backup))),
    path.toNamespacedPath(fs.realpathSync(root)), 'Backup must be beside this disposable installation');
  assert(path.basename(receipt.backup).startsWith('.rocket-previous-'));
  verifyTree(target, next); verifyTree(receipt.backup, old);
  pass('Old packaged UI downloaded and installed the new ZIP, launched its desktop, and retained the byte-exact old release');
  await closeDesktop();
  const untouched = cacheNames();
  const following = next.version.split('.').map(Number); following[2]++;
  offer(following.join('.'), invalid);
  await startHost(next.version); await review(); await click('update-download');
  await waitState(s => s.updates.status === 'error' && !s.updates.running, 'rejected incomplete bundle');
  assert.deepEqual(cacheNames(), untouched);
  pass('Verification failure after extraction removed only the failed attempt');
  offer(following.join('.'), next); await review(); await click('update-download');
  await until(() => cacheNames().some(n => !untouched.includes(n) && fs.existsSync(path.join(cache,n,'bundle','CHANGELOG.md'))),
    'actual partial extraction', packageTimeout);
  await click('update-stop');
  await waitState(s => s.updates.status === 'cancelled' && !s.updates.running, 'extraction cancellation');
  assert.deepEqual(cacheNames(), untouched);
  assert.equal(fs.readFileSync(path.join(retained,'marker'),'utf8'), 'keep');
  assert.deepEqual(repoState(), before);
  verifyTree(target, next); verifyTree(receipt.backup, old);
  pass('Cancellation removed the ZIP and partial tree and preserved prior jobs, the rollback copy and the dirty repository');
  report.oldFiles = old.files.size; report.newFiles = next.files.size;
}
(async () => {
  try { await test(); report.passed = true; }
  catch (e) { report.error = e.stack; console.error(e.message); }
  finally {
    for (const clean of [shutdown, closeDesktop]) {
      try { await clean(); }
      catch (e) {
        report.passed = false;
        (report.cleanupErrors ??= []).push(e.message);
        if (clean === shutdown && service?.exitCode === null) service.kill();
      }
    }
    if (profile) fs.rmSync(path.join(profile, 'access-token'), {force: true});
    if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
    if (report.passed) {
      checkFixture();
      fs.rmSync(root, {recursive: true, maxRetries: 3, retryDelay: 300});
      report.disposableFolderRemoved = !fs.existsSync(root);
    } else { report.fixture = root; process.exitCode = 1; }
    fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n', {flag: 'wx'});
    console.log('Report: ' + output);
  }
})().catch(e => { console.error(e); process.exitCode = 1; });
