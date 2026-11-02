// Native Windows check of TGA image comparisons through authenticated localhost MCP, with
// synthetic owned repositories and hand-built TGA files (uncompressed and run-length encoded,
// a damaged one). No network, no writes by Rocket.
// Usage: node tools/tests/test-mcp-tga.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/tga-' + crypto.randomUUID().replaceAll('-', ''));
const profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/tga';
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [] };
fs.mkdirSync(root, { recursive: true }); fs.mkdirSync(path.join(root, 'empty-template')); fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env }; for (const key of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, { GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template') });
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const IMAGE = 'art/sky 🐦.tga', BROKEN = 'art/broken.TGA';

/// A 32-bit truecolor TGA of one colour (blue, green, red, alpha), top-left origin;
/// run-length encoded with one packet per row (rows of at most 128 pixels).
function tga(width, height, [b, g, r, a], rle = false) {
  const header = Buffer.alloc(18);
  header[2] = rle ? 10 : 2;
  header.writeUInt16LE(width, 12); header.writeUInt16LE(height, 14);
  header[16] = 32; header[17] = 0x28;
  const pixel = Buffer.from([b, g, r, a]);
  const rows = [];
  for (let y = 0; y < height; y++) rows.push(rle ? Buffer.concat([Buffer.from([0x80 | (width - 1)]), pixel]) : Buffer.concat(Array(width).fill(pixel)));
  return Buffer.concat([header, ...rows]);
}

function fixture() {
  const repo = path.join(root, 'PRIVATE_TGA'), hooks = path.join(root, 'hooks');
  fs.mkdirSync(repo); fs.mkdirSync(hooks);
  const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();
  const write = (file, body) => { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, body); };
  git(['init', '--initial-branch=' + branch]);
  for (const [key, value] of [['user.name', 'Rocket TGA Fixture'], ['user.email', 'rocket@example.invalid'], ['commit.gpgsign', 'false'], ['core.autocrlf', 'false'], ['core.hooksPath', hooks]]) git(['config', key, value]);
  write(IMAGE, tga(8, 6, [220, 120, 40, 255]));
  write(BROKEN, tga(4, 4, [0, 0, 255, 255]));
  git(['add', '-A']); git(['commit', '-m', 'PRIVATE_IMAGES']);
  fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
  // Working changes: a larger, half-transparent RLE image, and a truncated file.
  write(IMAGE, tga(10, 6, [40, 60, 230, 128], true));
  write(BROKEN, tga(4, 4, [0, 0, 255, 255]).subarray(0, 24));
  return { repo, git };
}

let client, service, state;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = text => { report.cases.push(text); console.error('PASS ' + text); };
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out; ' + JSON.stringify(tab()?.image_diff ?? state.notice));
    await delay(80);
  }
}
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading || t.search.running), 'Settled'); }
const control = id => state.controls.find(c => c.id === id);
const tab = () => state.tabs?.find(t => t.id === state.active_tab);
const img = () => tab().image_diff;
async function stable() { let last = '', count = 0; await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return count >= 3; }, 'Stable widget geometry'); }
async function click(target) { await stable(); assert(control(target)?.enabled, 'Missing/disabled ' + target); await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle(); }
async function capture(name) { await stable(); const result = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!result.isError); const image = result.content.find(c => c.type === 'image'); assert(image); fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64')); fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state, null, 2)); report.captures = [...(report.captures || []), name]; }
async function resize(width, height) { await client.tool('rocket_resize', { width, height }); await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending); await settle(); }
async function closeWindow() { await client.tool('rocket_close'); await until(s => ['closed', 'not_started'].includes(s.window) || (s.dialog?.kind === 'quit' && !s.input_pending), 'Close window or exit confirmation'); if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'Closed window'); } }
async function change(name) {
  await click('view-changes');
  const id = 'change-unstaged-' + Buffer.from(name).toString('hex');
  for (let n = 0; !control(id) && n < 20; n++) { await client.tool('rocket_scroll', { target: 'changes-scroll', direction: 'down', pixels: 300, expected_tab: state.active_tab }); await settle(); }
  await click(id);
  await until(s => !img().running && (img().loaded || img().error), 'Image comparison');
}

async function main() {
  try {
    const f = fixture();
    const before = { head: f.git(['rev-parse', 'HEAD']), index: hash(path.join(f.repo, '.git/index')), image: hash(path.join(f.repo, IMAGE)) };
    const exe = path.resolve('target/release/rocket-mcp.exe'); report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer(); await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve)); const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const log = fs.openSync(path.join(root, 'service.log'), 'w'); service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', log] }); fs.closeSync(log); client = new RocketHttpClient(profile);
    for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i === 100) throw e; await delay(100); } }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await client.tool('rocket_open', { path: f.repo }); await settle(); await resize(1480, 920);
    if (control('console-close')) await click('console-close');

    await change(IMAGE);
    assert(img().loaded, JSON.stringify(img()));
    assert.deepEqual([img().before.format, img().before.width, img().before.height, img().before.alpha], ['TGA', 8, 6, false]);
    assert.deepEqual([img().after.format, img().after.width, img().after.height, img().after.alpha], ['TGA', 10, 6, true]);
    assert.equal(img().pixels, null, 'Side-by-side preview computes no pixel differences');
    await capture('tga-side-by-side');
    for (const mode of ['overlay', 'pixels']) { await click('image-mode-' + mode); assert.equal(img().mode, mode); await capture('tga-' + mode); }
    assert(img().pixels && img().pixels.changed > 0, JSON.stringify(img().pixels));
    report.pixels = img().pixels;
    passed('An uncompressed TGA and its run-length-encoded, half-transparent replacement compare: 8×6 and 10×6, alpha detected, changed pixels counted, every mode drawn');

    await change(BROKEN);
    assert(!img().loaded && /damaged/.test(img().error || ''), JSON.stringify(img()));
    await capture('tga-damaged');
    passed('A truncated TGA reports a damaged image instead of a picture');

    await resize(780, 620); await change(IMAGE); assert(img().loaded); await capture('tga-narrow'); await resize(1480, 920);
    assert.equal(f.git(['rev-parse', 'HEAD']), before.head); assert.equal(hash(path.join(f.repo, '.git/index')), before.index);
    assert.equal(hash(path.join(f.repo, IMAGE)), before.image);
    await closeWindow(); await client.raw('POST', '/shutdown');
    const records = fs.readdirSync(root).filter(n => n.startsWith('mcp-logs-')).flatMap(n => fs.readdirSync(path.join(root, n)).filter(x => /^rocket(?:\.\d+)?\.jsonl$/.test(x)).flatMap(x => fs.readFileSync(path.join(root, n, x), 'utf8').trim().split(/\r?\n/).filter(Boolean).map(JSON.parse)));
    assert(records.length); assert(records.every(r => r.dropped_before === 0)); const payload = JSON.stringify(records);
    for (const secret of ['PRIVATE', 'sky', 'broken']) assert(!payload.includes(secret), 'Diagnostic leak ' + secret);
    for (const op of ['image-decode-before', 'image-decode-after', 'image-difference']) {
      const starts = records.filter(r => r.event === 'request_start' && r.operation === op);
      assert(starts.length, 'Missing ' + op);
      for (const s of starts) assert(records.some(r => r.event === 'request_end' && r.session === s.session && r.request === s.request), 'Unpaired ' + op);
    }
    report.log_records = records.length; passed('Files, HEAD and index unchanged at 1480×920 and 780×620; paired decode/difference requests, zero dropped events, no names or paths');
    report.passed = true;
  } catch (error) { report.failure = error.stack; if (client) try { await capture('failure'); } catch {} throw error; }
  finally {
    if (service && service.exitCode === null) { try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {} await delay(200); if (service.exitCode === null) service.kill(); }
    fs.writeFileSync('work/tga-last-result.txt', path.join(root, 'result.json')); fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2)); fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
  }
}
main().catch(() => { process.exitCode = 1; });
