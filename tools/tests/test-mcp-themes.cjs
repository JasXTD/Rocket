// Native acceptance for themes through authenticated localhost MCP: Preferences → Appearance
// and View → Appearance switch Default, Dark, Light, System and Custom, and state.appearance
// reports each. Custom covers base, accent swatch/colour code/From picture, the four built-in
// pictures, a picture chosen in the fixture-scoped picker (refused when it is not a picture),
// size, position and Visibility (a click moves its slider to the centre, 35%, and is refused
// while it is off), and survives closing and reopening the window. One disposable repository and
// isolated settings; one screenshot per theme and Custom variant.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), zlib = require('node:zlib');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const root = path.resolve('work/mcp-fixtures/themes-' + crypto.randomUUID().replaceAll('-', ''));
const repo = path.join(root, 'repo'), profile = path.join(root, 'http-profile'), branch = 'rocket-ui-test/themes';
const exe = path.resolve(process.argv[2] || 'target/release/rocket-mcp.exe');
const version = fs.readFileSync('Cargo.toml', 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/)[1];
const report = { passed: false, version, root, cases: [], screenshots: [] };
// The fixture's own pictures sit in the root, where the scoped picker starts.
const picture = path.join(root, 'rocket-theme-picture.png'), broken = path.join(root, 'rocket-theme-broken.png');
const FILE = 'src/orbit.rs';
// The colour the Astronaut picture was drawn around, which From picture uses.
const ASTRONAUT = '#F472B6';
fs.mkdirSync(repo, { recursive: true });
fs.mkdirSync(path.join(root, 'empty-template'));
fs.writeFileSync(path.join(root, 'empty.config'), '');
const env = { ...process.env };
for (const key of Object.keys(env)) if (/^(GIT_|SSH_)|_PROXY$/i.test(key)) delete env[key];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'),
  GIT_CONFIG_NOSYSTEM: '1', GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0',
  GIT_TEMPLATE_DIR: path.join(root, 'empty-template')
});
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const passed = message => { report.cases.push(message); console.error('PASS ' + message); };
const git = args => cp.execFileSync('git', ['-C', repo, ...args], { env, windowsHide: true, encoding: 'utf8' }).trimEnd();

// A 960x600 picture: a vivid green planet on a dark sky, so From picture finds green.
function png(width, height, pixel) {
  const stride = width * 4 + 1, raw = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) raw.set([...pixel(x, y), 255], y * stride + 1 + x * 4);
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type, 'ascii'), data]), length = Buffer.alloc(4), crc = Buffer.alloc(4);
    length.writeUInt32BE(data.length); crc.writeUInt32BE(zlib.crc32(body));
    return Buffer.concat([length, body, crc]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0); header.writeUInt32BE(height, 4); header.set([8, 6, 0, 0, 0], 8);
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', header),
    chunk('IDAT', zlib.deflateSync(raw)), chunk('IEND', Buffer.alloc(0))]);
}
fs.writeFileSync(picture, png(960, 600, (x, y) => {
  const d = Math.hypot(x - 700, y - 300);
  if (d < 210) { const shade = 1 - 0.45 * d / 210; return [10, Math.round(200 * shade + 30), Math.round(90 * shade)]; }
  return [18, 22 + Math.round(y / 40), 36 + Math.round(x / 60)];
}));
fs.writeFileSync(broken, 'This file is not a picture.\n');

git(['init', '--initial-branch=' + branch]);
for (const [key, value] of [['user.name', 'Rocket Fixture'], ['user.email', 'rocket@example.invalid'],
  ['core.hooksPath', path.join(root, 'empty-template')], ['core.autocrlf', 'false'], ['commit.gpgsign', 'false']]) git(['config', key, value]);
const source = lines => lines.join('\n') + '\n';
fs.mkdirSync(path.join(repo, 'src'));
fs.writeFileSync(path.join(repo, FILE), source(['/// One body orbiting another.', 'pub struct Orbit {', '    pub radius: f64,',
  '    pub period: f64,', '}', '', 'impl Orbit {', '    pub fn speed(&self) -> f64 {', '        2.0 * std::f64::consts::PI * self.radius / self.period',
  '    }', '}']));
git(['add', '.']); git(['commit', '-m', 'Orbit fixture']);
fs.writeFileSync(path.join(repo, FILE), source(['/// One body orbiting another, in kilometres and seconds.', 'pub struct Orbit {',
  '    pub radius: f64,', '    pub period: f64,', '    pub name: String,', '}', '', 'impl Orbit {', '    /// Mean orbital speed.',
  '    pub fn speed(&self) -> f64 {', '        std::f64::consts::TAU * self.radius / self.period', '    }', '}']));
fs.writeFileSync(path.join(repo, '.git/rocket-ui-fixture'), branch);
const preserved = () => ({ head: git(['rev-parse', 'HEAD']), index: git(['ls-files', '--stage', '-z']),
  config: hash(path.join(repo, '.git/config')), file: hash(path.join(repo, FILE)), picture: hash(picture) });
const before = preserved();

// The Windows app mode System follows, read-only; null when Windows does not record one.
function windowsMode() {
  try {
    const out = cp.execFileSync('reg', ['query', 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize',
      '/v', 'AppsUseLightTheme'], { encoding: 'utf8', windowsHide: true });
    const value = out.match(/AppsUseLightTheme\s+REG_DWORD\s+0x([0-9a-f]+)/i);
    return value ? (parseInt(value[1], 16) ? 'light' : 'dark') : null;
  } catch { return null; }
}

let client, service, state;
async function until(predicate, label = 'UI state') {
  const start = performance.now();
  for (;;) {
    state = await client.tool('rocket_state');
    if (predicate(state)) return state;
    assert(performance.now() - start < 45000, label + ' timed out: ' + JSON.stringify({ dialog: state.dialog?.kind, appearance: state.appearance, notice: state.notice }));
    await delay(80);
  }
}
const control = id => state.controls.find(c => c.id === id);
const appearance = () => state.appearance;
async function settle() { return until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading), 'settled'); }
async function stable() {
  let last = '', count = 0;
  await until(s => { const now = JSON.stringify(s.controls); count = now === last ? count + 1 : 0; last = now; return !s.input_pending && count >= 3; }, 'Stable control geometry');
}
async function scroll(target, direction, pixels) {
  await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab });
  await settle(); await stable();
}
// An Appearance control fully inside the Preferences page's scroll viewport (a clipped
// control touches its edge); any other control once it is present.
const onPage = id => id.startsWith('theme-') && Boolean(control('prefs-scroll'));
function shown(id) {
  const c = control(id), view = control('prefs-scroll')?.rect;
  return Boolean(c && (!onPage(id) || c.rect[1] > view[1] + 0.5 && c.rect[3] < view[3] - 0.5));
}
async function reveal(id) {
  await stable();
  if (shown(id) || !onPage(id)) return;
  for (let i = 0; i < 3 && !shown(id); i++) await scroll('prefs-scroll', 'up', 2000);
  for (let i = 0; i < 25 && !shown(id); i++) await scroll('prefs-scroll', 'down', 200);
  assert(shown(id), 'Cannot scroll ' + id + ' into view');
}
async function click(target) {
  await reveal(target); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_click', { target, expected_tab: state.active_tab }); await settle();
}
async function type(target, text) {
  await reveal(target); assert(control(target)?.enabled, 'Missing/disabled ' + target);
  await client.tool('rocket_type', { target, text, expected_tab: state.active_tab }); await settle();
}
async function resize(width, height) {
  await client.tool('rocket_resize', { width, height });
  await until(s => s.viewport[0] === width && s.viewport[1] === height && !s.input_pending, 'resize'); await settle();
}
async function capture(name) {
  await stable();
  const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} });
  assert(!r.isError); const image = r.content.find(c => c.type === 'image'); assert(image);
  fs.writeFileSync(path.join(root, name + '.png'), Buffer.from(image.data, 'base64'));
  fs.writeFileSync(path.join(root, name + '.json'), JSON.stringify(state.appearance, null, 2));
  report.screenshots.push(name + '.png');
}
// Waits until the theme shows: the palette applied and, for a Custom picture, its texture.
async function showing(theme, palette, label) {
  return until(s => s.appearance?.theme === theme && s.appearance.palette === palette
    && (!s.appearance.custom || s.appearance.custom.picture === 'none' || s.appearance.custom.picture_shown), label);
}
// The theme's palette under System: Windows' mode as Rocket was told it, Dark without one.
const systemPalette = () => appearance().windows_mode === 'light' ? 'light' : 'dark';
function noPaths() {
  const text = JSON.stringify(appearance());
  for (const part of [path.basename(root), 'rocket-ui-state', 'http-profile']) assert(!text.includes(part), 'appearance names a path: ' + text);
}
async function open() {
  await client.tool('rocket_open', { path: repo });
  await settle(); await resize(1440, 960);
  if (control('console-close')) await click('console-close');
}
async function closeWindow() {
  if (state?.dialog) await click(state.dialog.kind === 'other' ? 'dialog-submit' : 'dialog-cancel');
  await client.tool('rocket_close');
  await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit' && !s.input_pending, 'closing');
  if (state.dialog?.kind === 'quit') {
    await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab });
    await until(s => ['closed', 'not_started'].includes(s.window), 'closed');
  }
}
async function pick(name) {
  await click(control('theme-picture-own') ? 'theme-picture-own' : 'theme-picture-choose');
  await until(s => s.controls.some(c => c.id === 'picker-choose'), 'picture picker');
  assert(!control('picker-choose').enabled, 'nothing is chosen yet');
  await type('picker-filter', name);
  await until(s => s.controls.some(c => c.id === 'picker-row-0') && !s.controls.some(c => c.id === 'picker-row-1'), 'only ' + name + ' listed');
  await click('picker-row-0'); await click('picker-choose');
  await until(s => !s.controls.some(c => c.id === 'picker-choose') && !s.appearance.importing, 'picture copied');
}
// View → Appearance, then the theme; the menu closes on the click.
async function viewTheme(theme) {
  await click('menu-view'); await click('view-appearance');
  await until(s => s.controls.some(c => c.id === 'view-theme-' + theme), 'View → Appearance');
  for (const id of ['default', 'dark', 'light', 'system', 'custom']) assert(control('view-theme-' + id)?.enabled, 'View → Appearance lists ' + id);
  await click('view-theme-' + theme);
  if (control('view-appearance')) await click('menu-view');
  assert(!control('view-appearance'), 'the View menu closed');
}

async function run() {
  try {
    report.executable_sha256 = hash(exe);
    cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
    const listener = net.createServer();
    await new Promise(resolve => listener.listen(0, '127.0.0.1', resolve));
    const port = listener.address().port; await new Promise(resolve => listener.close(resolve));
    fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
    const fd = fs.openSync(path.join(root, 'service.log'), 'w');
    service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', 'ignore', fd] }); fs.closeSync(fd);
    client = new RocketHttpClient(profile);
    for (let i = 0;; i++) {
      try { assert.equal((await client.raw('GET', '/health')).status, 200); break; }
      catch (e) { if (i === 100) throw e; await delay(100); }
    }
    assert.equal((await client.initialize()).serverInfo.version, version);
    await open();
    const registry = windowsMode();
    report.windows_registry_mode = registry;
    assert.deepEqual({ theme: appearance().theme, palette: appearance().palette, custom: appearance().custom, importing: appearance().importing },
      { theme: 'default', palette: 'default', custom: null, importing: false });
    assert.match(appearance().accent, /^#[0-9A-F]{6}$/);
    if (registry) assert.equal(appearance().windows_mode, registry, 'Rocket is told Windows\' app mode');
    passed('state.appearance reports the Default theme without Custom settings');

    // Preferences → Appearance.
    await click('menu-edit'); await click('preferences'); await click('pref-page-appearance');
    await until(s => s.preferences?.page === 'appearance', 'Appearance page');
    for (const id of ['default', 'dark', 'light', 'system', 'custom']) assert(control('theme-' + id)?.enabled, 'tile ' + id);
    assert(!control('theme-base-default'), 'Custom cards show only for Custom');
    await click('theme-dark'); await showing('dark', 'dark', 'Dark'); await capture('01-preferences-dark');
    await click('theme-light'); await showing('light', 'light', 'Light'); await capture('02-preferences-light');
    await click('theme-system'); await showing('system', systemPalette(), 'System'); await capture('03-preferences-system');
    passed('Preferences → Appearance switches Dark, Light and System (follows Windows: ' + appearance().palette + ')');

    await click('theme-custom'); await showing('custom', 'default', 'Custom');
    assert.deepEqual(appearance().custom, { base: 'default', accent: 'picture', picture: 'astronaut', picture_name: null, size: 'fill',
      position: 'right', visibility: 30, picture_shown: true, picture_preparing: false, picture_problem: null });
    assert.equal(appearance().accent, ASTRONAUT, 'From picture takes the astronaut\'s colour');
    await capture('04-preferences-custom-astronaut');
    passed('Custom starts from Default with the Astronaut picture and its colour');

    await click('theme-base-light'); await showing('custom', 'light', 'Custom from Light');
    await click('theme-accent-green'); await until(s => s.appearance.custom.accent === '#34D399', 'Green swatch');
    await type('theme-accent-code', '#E85D04'); await until(s => s.appearance.custom.accent === '#E85D04', 'Colour code');
    await click('theme-picture-planet'); await click('theme-size-fit'); await click('theme-position-left');
    await until(s => s.appearance.custom.picture === 'planet' && s.appearance.custom.size === 'fit' && s.appearance.custom.position === 'left'
      && s.appearance.custom.picture_shown, 'Planet, Fit and Left');
    // A slider senses drags only, so a click on one releases a frame after its press; the
    // press moves Visibility to the clicked point, the centre of 10–60%.
    assert.equal(appearance().custom.visibility, 30);
    await click('theme-visibility');
    await until(s => s.appearance.custom.visibility === 35 && s.appearance.custom.picture_shown, 'Visibility at 35%');
    assert.equal(appearance().custom.accent, '#E85D04');
    await capture('05-preferences-custom-light-planet');
    passed('Custom takes a base, a swatch, a colour code, a picture, size, position and Visibility (30% to 35%)');

    for (const [key, number] of [['space-cat', '06'], ['stars', '07']]) {
      await click('theme-picture-' + key);
      await until(s => s.appearance.custom.picture === key && s.appearance.custom.picture_shown, key);
      await capture(number + '-preferences-' + key);
    }
    assert.equal(appearance().custom.visibility, 35, 'the slider click released: later clicks do not drag it');
    await click('theme-accent-picture');
    await until(s => s.appearance.custom.accent === 'picture' && s.appearance.custom.picture_shown, 'From picture');
    await click('theme-picture-none');
    await until(s => s.appearance.custom.picture === 'none' && !s.appearance.custom.picture_shown, 'No picture');
    for (const id of ['theme-size-fill', 'theme-position-centre', 'theme-visibility']) {
      await reveal(id); assert(!control(id).enabled, id + ' needs a picture');
    }
    await assert.rejects(client.tool('rocket_click', { target: 'theme-visibility', expected_tab: state.active_tab }),
      /Control is disabled/);
    await settle(); assert.equal(appearance().custom.visibility, 35, 'a refused click changes nothing');
    passed('All four built-in pictures show; None turns size, position and visibility off, and a click on Visibility is refused');

    await pick('rocket-theme-broken');
    await until(s => s.appearance.import_error === 'Choose a PNG, JPG or WebP picture.', 'refused file');
    assert.equal(appearance().custom.picture, 'none');
    assert(!fs.existsSync(path.join(root, 'rocket-ui-state/themes')) || fs.readdirSync(path.join(root, 'rocket-ui-state/themes')).length === 0, 'nothing is kept');
    passed('A file that is not a picture is refused and nothing is kept');

    await pick('rocket-theme-picture');
    await until(s => s.appearance.custom.picture === 'own' && s.appearance.custom.picture_shown && !s.appearance.import_error, 'own picture');
    const name = appearance().custom.picture_name;
    assert.match(name, /^picture-[0-9a-f]{16}\.png$/);
    assert.deepEqual(fs.readdirSync(path.join(root, 'rocket-ui-state/themes')), [name], 'one copy in Rocket\'s storage');
    const [r, g, b] = appearance().accent.match(/[0-9A-F]{2}/g).map(h => parseInt(h, 16));
    assert(g > r && g > b, 'From picture takes the picture\'s green: ' + appearance().accent);
    await reveal('theme-picture-own'); assert(!control('theme-picture-choose'), 'Choose image becomes Your picture');
    noPaths();
    await capture('08-preferences-own-picture');
    passed('A picture chosen in the scoped picker is copied into Rocket\'s storage and named, not pathed (' + name + ')');
    const custom = appearance().custom;
    await click('dialog-submit'); await until(s => !s.dialog, 'Preferences closed');

    // View → Appearance over the main window, with a diff showing.
    await click('view-changes'); await click('change-unstaged-' + Buffer.from(FILE).toString('hex'));
    await until(s => s.tabs.find(t => t.id === s.active_tab)?.patch_files?.length, 'diff');
    for (const [theme, number] of [['default', '09'], ['dark', '10'], ['light', '11'], ['system', '12']]) {
      await viewTheme(theme);
      await showing(theme, theme === 'system' ? systemPalette() : theme, 'View → Appearance ' + theme);
      assert.equal(appearance().custom, null);
      await capture(number + '-view-' + theme);
    }
    await viewTheme('custom'); await showing('custom', 'light', 'View → Appearance custom');
    assert.deepEqual(appearance().custom, custom, 'Custom keeps its settings while other themes show');
    await capture('13-view-custom');
    passed('View → Appearance switches all five themes over the main window; Custom comes back as it was left');

    await closeWindow(); await open();
    await showing('custom', 'light', 'reopened Custom');
    assert.deepEqual(appearance().custom, custom, 'the Custom theme and its picture are restored');
    noPaths();
    await capture('14-reopened-custom');
    passed('The theme and the copied picture survive closing and reopening the window');

    assert.deepEqual(preserved(), before, 'repository, index, config and the source picture are unchanged');
    passed('Theme changes leave the repository and the chosen picture file untouched');
    report.passed = true;
  } catch (error) {
    report.error = error.stack;
    if (client) try { await capture('failure'); } catch {}
    throw error;
  } finally {
    fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2));
    fs.writeFileSync(path.join(root, 'last-state.json'), JSON.stringify(state ?? null, null, 2));
    console.log(JSON.stringify({ passed: report.passed, cases: report.cases.length, screenshots: report.screenshots.length, root }, null, 2));
    if (service && service.exitCode === null) {
      try { await closeWindow(); await client.raw('POST', '/shutdown'); } catch {}
      await delay(150); if (service.exitCode === null) service.kill();
    }
    if (client) await client.finish();
  }
}
run().catch(error => { console.error(error); process.exitCode = 1; });
