// Takes the website's screenshots from the real window through localhost MCP, in a throwaway demo
// repository with made-up authors. Nothing here pushes or touches a real repository.
// Claim the lock in COORDINATION.md first, and leave the Rocket window alone while it runs (about 10
// minutes). Usage: node tools/website/capture.cjs [output folder] [steps to retake], then
// node tools/website/media.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const project = path.resolve(__dirname, '../..');
const root = path.join(project, 'work/mcp-fixtures/site-' + crypto.randomUUID().replaceAll('-', ''));
const out = path.resolve(process.argv[2] || path.join(project, 'work/website-shots'));
const only = new Set(process.argv.slice(3));
const exe = path.join(project, 'target/release/rocket-mcp.exe');
const profile = path.join(root, 'http-profile');
const remote = path.join(root, 'remote', 'orbit-planner.git');
const repoA = path.join(root, 'a', 'orbit-planner'), repoB = path.join(root, 'b', 'orbit-planner'), repoC = path.join(root, 'c', 'orbit-planner');
const marker = path.join(root, 'rocket-private-state'), MARK = 'rocket-ui-test/site';
const W = 1920, H = 1017;

for (const d of [out, path.join(root, 'empty-template'), repoA, repoB, path.dirname(repoC), path.dirname(remote)]) fs.mkdirSync(d, { recursive: true });
for (const d of ['home', 'local', 'roaming']) fs.mkdirSync(path.join(root, d));
fs.writeFileSync(path.join(root, 'empty.config'), '');
fs.writeFileSync(marker, 'synthetic-content-only-v1\n');
const env = { ...process.env };
for (const k of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|CODEX_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k)) delete env[k];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1',
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'),
  USERPROFILE: path.join(root, 'home'), LOCALAPPDATA: path.join(root, 'local'), APPDATA: path.join(root, 'roaming')
});
const delay = ms => new Promise(r => setTimeout(r, ms));

// ---- The demo project: orbit-planner, a small Rust library written for these screenshots.
const AUTHORS = { ada: 'Ada Example', sam: 'Sam Example', kai: 'Kai Example' };
let clock = Date.UTC(2026, 8, 7, 9, 12);
function git(repo, args, who = 'ada') {
  clock += 1000 * 60 * (47 + (clock / 60000 % 311));
  const stamp = new Date(clock).toISOString().replace(/\.\d+Z$/, ' +0000');
  const name = AUTHORS[who], mail = who + '@example.invalid';
  return cp.execFileSync('git', ['-C', repo, ...args], { env: { ...env, GIT_AUTHOR_NAME: name, GIT_AUTHOR_EMAIL: mail, GIT_COMMITTER_NAME: name,
    GIT_COMMITTER_EMAIL: mail, GIT_AUTHOR_DATE: stamp, GIT_COMMITTER_DATE: stamp }, windowsHide: true, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trimEnd();
}
function write(repo, file, text) { const p = path.join(repo, file); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, text); }
const commit = (repo, who, message) => { git(repo, ['add', '-A'], who); git(repo, ['commit', '-m', message], who); };

const CARGO = v => `[package]\nname = "orbit-planner"\nversion = "${v}"\nedition = "2024"\n\n[[bin]]\nname = "plan"\npath = "src/bin/plan.rs"\n`;
const README = extra => `# orbit-planner\n\nPlan burns between circular orbits around one body.\n\n\`\`\`\ncargo run --bin plan -- 400 35786\n\`\`\`\n\nAltitudes are in kilometres.${extra}\n`;
const ORBIT = `//! Circular orbits, in kilometres and seconds.
use std::f64::consts::TAU;

/// Gravitational parameter of Earth, km³/s².
pub const MU_EARTH: f64 = 398_600.4418;
/// Mean radius of Earth, km.
pub const RADIUS_EARTH: f64 = 6_371.0;

/// A circular orbit around one body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    /// Distance from the centre of the body, km.
    pub radius: f64,
    /// Gravitational parameter of the body, km³/s².
    pub mu: f64,
}

impl Orbit {
    /// An orbit at \`altitude\` kilometres above Earth.
    pub fn around_earth(altitude: f64) -> Self {
        Self { radius: RADIUS_EARTH + altitude, mu: MU_EARTH }
    }

    /// Orbital speed, km/s.
    pub fn speed(&self) -> f64 {
        (self.mu / self.radius).sqrt()
    }

    /// Time for one orbit, seconds.
    pub fn period(&self) -> f64 {
        TAU * (self.radius.powi(3) / self.mu).sqrt()
    }
}
`;
const TRANSFER = `//! Burns between two circular orbits around the same body.
use crate::orbit::Orbit;
use std::f64::consts::PI;

/// The two burns of a transfer and the coast between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transfer {
    /// Speed change at departure, km/s.
    pub departure_burn: f64,
    /// Speed change at arrival, km/s.
    pub arrival_burn: f64,
    /// Coast between the burns, seconds.
    pub coast: f64,
}

impl Transfer {
    /// Total speed change, km/s.
    pub fn total(&self) -> f64 {
        self.departure_burn.abs() + self.arrival_burn.abs()
    }
}

/// A Hohmann transfer: half an ellipse that touches both orbits.
pub fn hohmann(from: Orbit, to: Orbit) -> Transfer {
    let semi_major = (from.radius + to.radius) / 2.0;
    let mu = from.mu;
    let depart = (mu * (2.0 / from.radius - 1.0 / semi_major)).sqrt();
    let arrive = (mu * (2.0 / to.radius - 1.0 / semi_major)).sqrt();
    Transfer {
        departure_burn: depart - from.speed(),
        arrival_burn: to.speed() - arrive,
        coast: PI * (semi_major.powi(3) / mu).sqrt(),
    }
}
`;
const PLAN = `use orbit_planner::{hohmann, Orbit};

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<f64>());
    let (Some(Ok(from)), Some(Ok(to))) = (args.next(), args.next()) else {
        eprintln!("usage: plan <from-km> <to-km>");
        std::process::exit(2);
    };
    let transfer = hohmann(Orbit::around_earth(from), Orbit::around_earth(to));
    println!("departure {:.3} km/s", transfer.departure_burn);
    println!("arrival   {:.3} km/s", transfer.arrival_burn);
    println!("total     {:.3} km/s", transfer.total());
    println!("coast     {:.0} s", transfer.coast);
}
`;
const LIB = `pub mod orbit;\npub mod transfer;\n\npub use orbit::{Orbit, MU_EARTH};\npub use transfer::{hohmann, Transfer};\n`;
const TESTS = `use orbit_planner::{hohmann, Orbit};

#[test]
fn low_orbit_to_geostationary() {
    let transfer = hohmann(Orbit::around_earth(400.0), Orbit::around_earth(35_786.0));
    assert!((transfer.total() - 3.89).abs() < 0.01);
}
`;

function build() {
  const r = repoA;
  git(r, ['init', '--initial-branch=main']);
  for (const [k, v] of [['core.autocrlf', 'false'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')],
    ['merge.conflictStyle', 'diff3'], ['user.name', AUTHORS.ada], ['user.email', 'ada@example.invalid']]) git(r, ['config', k, v]);
  write(r, 'Cargo.toml', CARGO('0.1.0')); write(r, 'README.md', README('')); write(r, 'src/orbit.rs', ORBIT.replace(/\n    \/\/\/ Time for one orbit[\s\S]*?\n    }\n}\n$/, '\n}\n').replace('use std::f64::consts::TAU;\n\n', '\n'));
  write(r, 'src/lib.rs', 'pub mod orbit;\n'); commit(r, 'ada', 'Create orbit-planner with circular orbits'); git(r, ['tag', 'v0.1.0']);
  write(r, 'src/orbit.rs', ORBIT); commit(r, 'ada', 'Add the orbital period');
  write(r, 'src/transfer.rs', TRANSFER); write(r, 'src/lib.rs', LIB); commit(r, 'sam', 'Add Hohmann transfers');
  write(r, 'tests/transfer.rs', TESTS); commit(r, 'sam', 'Test low orbit to geostationary');
  git(r, ['checkout', '-b', 'fix/coast-rounding']);
  write(r, 'src/bin/plan.rs', PLAN); commit(r, 'kai', 'Print the coast in whole seconds');
  git(r, ['checkout', 'main']);
  write(r, 'README.md', README('\n\nThe plan command prints each burn in km/s.')); commit(r, 'ada', 'Explain the plan command');
  git(r, ['merge', '--no-ff', '-m', "Merge branch 'fix/coast-rounding'", 'fix/coast-rounding'], 'ada');
  write(r, 'Cargo.toml', CARGO('0.2.0')); commit(r, 'ada', 'Release 0.2.0'); git(r, ['tag', 'v0.2.0']);
  git(r, ['checkout', '-b', 'feature/gravity-loss']);
  write(r, 'src/transfer.rs', TRANSFER.replace('use std::f64::consts::PI;\n', 'use std::f64::consts::PI;\n\n/// Share of extra speed lost to gravity during a burn.\nconst GRAVITY_LOSS_MARGIN: f64 = 1.02;\n')
    .replace('    /// Total speed change, km/s.\n    pub fn total(&self) -> f64 {\n        self.departure_burn.abs() + self.arrival_burn.abs()\n',
      '    /// Total speed change, km/s, with a margin for gravity losses.\n    pub fn total(&self) -> f64 {\n        (self.departure_burn.abs() + self.arrival_burn.abs()) * GRAVITY_LOSS_MARGIN\n')
    .replace('coast: PI * (semi_major.powi(3) / mu).sqrt(),', 'coast: Orbit { radius: semi_major, mu }.period() / 2.0,').replace('use std::f64::consts::PI;\n\n', '\n'));
  commit(r, 'sam', 'Add a margin for gravity losses');
  write(r, 'tests/transfer.rs', TESTS.replace('3.89', '3.97')); commit(r, 'sam', 'Update the expected total');
  git(r, ['checkout', 'main']);
  write(r, 'README.md', README('\n\nThe plan command prints each burn in km/s.\n\nAdd `--json` for scripts.')); commit(r, 'kai', 'Mention JSON output');
  git(r, ['merge', '--no-ff', '-m', "Merge branch 'feature/gravity-loss'", 'feature/gravity-loss'], 'ada');
  write(r, 'Cargo.toml', CARGO('0.3.0')); commit(r, 'ada', 'Release 0.3.0'); git(r, ['tag', 'v0.3.0']);
  git(r, ['init', '--bare', '--initial-branch=main', remote]);
  git(r, ['remote', 'add', 'origin', remote]); git(r, ['push', '-u', 'origin', 'main', '--tags']);
  git(r, ['checkout', '-b', 'feature/moon-orbits', 'v0.2.0']);
  write(r, 'src/orbit.rs', ORBIT.replace('/// Mean radius of Earth, km.', '/// Gravitational parameter of the Moon, km³/s².\npub const MU_MOON: f64 = 4_902.8;\n/// Mean radius of Earth, km.'));
  commit(r, 'kai', 'Add the Moon');
  git(r, ['checkout', 'main']);
  // feature/plane-change starts before gravity-loss landed, so merging it into main conflicts.
  git(r, ['checkout', '-b', 'feature/plane-change', 'v0.2.0']);
  const planed = TRANSFER.replace('    /// Speed change at arrival, km/s.\n    pub arrival_burn: f64,\n', '    /// Speed change at arrival, km/s.\n    pub arrival_burn: f64,\n    /// Speed change to turn the orbital plane, km/s.\n    pub plane_burn: f64,\n')
    .replace('self.departure_burn.abs() + self.arrival_burn.abs()', 'self.departure_burn.abs() + self.arrival_burn.abs() + self.plane_burn.abs()')
    .replace('        arrival_burn: to.speed() - arrive,\n', '        arrival_burn: to.speed() - arrive,\n        plane_burn: 0.0,\n');
  write(r, 'src/transfer.rs', planed); commit(r, 'kai', 'Add a plane burn to Transfer');
  write(r, 'src/transfer.rs', planed.replace(/\n\/\/\/ A Hohmann transfer/, `\n/// Burn that turns an orbit by \`degrees\` at the same speed.
pub fn plane_change(orbit: Orbit, degrees: f64) -> f64 {
    2.0 * orbit.speed() * (degrees.to_radians() / 2.0).sin()
}

/// A Hohmann transfer`)); commit(r, 'kai', 'Add plane_change');
  const merged = fs.readFileSync(path.join(r, 'src/transfer.rs'), 'utf8');
  // Repository B: main merging feature/plane-change, stopped on two conflicts in src/transfer.rs.
  cp.execFileSync('git', ['clone', '--quiet', '--no-hardlinks', r, repoB], { env, windowsHide: true, stdio: 'pipe' });
  git(repoB, ['checkout', 'main']);
  for (const [k, v] of [['core.autocrlf', 'false'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')],
    ['merge.conflictStyle', 'diff3'], ['user.name', AUTHORS.ada], ['user.email', 'ada@example.invalid']]) git(repoB, ['config', k, v]);
  git(repoB, ['fetch', '--quiet', 'origin', 'refs/heads/*:refs/remotes/origin/*']);
  git(repoB, ['branch', '-f', 'feature/plane-change', 'origin/feature/plane-change']);
  assert.throws(() => git(repoB, ['merge', 'feature/plane-change']));
  git(repoB, ['remote', 'set-url', 'origin', remote]);
  write(repoB, '.git/rocket-ui-fixture', MARK);
  // Repository C: on the marked branch, for the interactive rebase dialog (cropped to the dialog).
  cp.execFileSync('git', ['clone', '--quiet', '--no-hardlinks', r, repoC], { env, windowsHide: true, stdio: 'pipe' });
  for (const [k, v] of [['core.autocrlf', 'false'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')],
    ['user.name', AUTHORS.kai], ['user.email', 'kai@example.invalid']]) git(repoC, ['config', k, v]);
  git(repoC, ['fetch', '--quiet', 'origin', 'refs/heads/*:refs/remotes/origin/*']);
  git(repoC, ['checkout', '-b', MARK, 'origin/feature/plane-change']); git(repoC, ['branch', '-f', 'main', 'origin/main']); git(repoC, ['branch', 'release/0.2', 'v0.2.0']);
  write(repoC, 'src/transfer.rs', merged.replace('/// Burn that turns an orbit', '/// The burn that turns an orbit')); commit(repoC, 'kai', 'Fix the plane_change comment');
  write(repoC, 'tests/transfer.rs', TESTS + '\n#[test]\nfn no_turn_costs_nothing() {\n    let orbit = Orbit::around_earth(400.0);\n    assert_eq!(orbit_planner::transfer::plane_change(orbit, 0.0), 0.0);\n}\n'); commit(repoC, 'kai', 'Test plane_change');
  write(repoC, 'notes.md', 'Try the combined burn next.\n'); commit(repoC, 'kai', 'Notes for later');
  write(repoC, '.git/rocket-ui-fixture', MARK);
  // Repository A: the working change for the Changes page, on feature/plane-change.
  write(r, 'README.md', README('\n\nThe plan command prints each burn in km/s.\n\nPass an inclination in degrees to add a plane change.'));
  git(r, ['add', 'README.md']);
  write(r, 'src/transfer.rs', merged.replace('/// A Hohmann transfer: half an ellipse that touches both orbits.\npub fn hohmann(from: Orbit, to: Orbit) -> Transfer {',
    '/// A Hohmann transfer: half an ellipse that touches both orbits, in one plane.\npub fn hohmann(from: Orbit, to: Orbit) -> Transfer {\n    hohmann_with_plane_change(from, to, 0.0)\n}\n\n/// A Hohmann transfer that turns the plane at arrival, where the craft is slowest.\npub fn hohmann_with_plane_change(from: Orbit, to: Orbit, degrees: f64) -> Transfer {')
    .replace('        arrival_burn: to.speed() - arrive,\n        plane_burn: 0.0,\n', '        arrival_burn: to.speed() - arrive,\n        plane_burn: plane_change(to, degrees) * arrive / to.speed(),\n')
    .replace('    let mu = from.mu;\n', '    let mu = from.mu;\n    assert!(from.mu == to.mu, "both orbits need the same body");\n'));
  write(r, 'src/orbit.rs', ORBIT.replace('    /// Distance from the centre of the body, km.\n    pub radius: f64,\n', '    /// Distance from the centre of the body, km.\n    pub radius: f64,\n    /// Tilt against the reference plane, degrees.\n    pub inclination: f64,\n')
    .replace('Self { radius: RADIUS_EARTH + altitude, mu: MU_EARTH }', 'Self { radius: RADIUS_EARTH + altitude, inclination: 0.0, mu: MU_EARTH }'));
  write(r, 'tests/transfer.rs', TESTS + `\n#[test]\nfn a_ten_degree_plane_change_costs_extra() {\n    let from = Orbit::around_earth(400.0);\n    let to = Orbit::around_earth(35_786.0);\n    let turned = orbit_planner::transfer::hohmann_with_plane_change(from, to, 10.0);\n    assert!(turned.total() > hohmann(from, to).total());\n}\n`);
  write(r, 'docs/plane-change.md', '# Plane changes\n\nTurning the orbit costs less at arrival, where the craft moves slowest.\n');
  write(r, '.git/rocket-ui-fixture', MARK);
  toMark(r); toMark(repoB);
}

// The window only takes clicks on the marked test branch, and the page should show the real branch
// names. So: drive the page on the marked branch, flip HEAD from outside for the capture, flip back.
function toMark(repo) { git(repo, ['update-ref', 'refs/heads/' + MARK, 'HEAD']); git(repo, ['symbolic-ref', 'HEAD', 'refs/heads/' + MARK]); }
function toReal(repo, name) { git(repo, ['symbolic-ref', 'HEAD', 'refs/heads/' + name]); git(repo, ['update-ref', '-d', 'refs/heads/' + MARK]); }

// ---- Driving the window.
let state, client, service;
const report = { shots: [], failed: [], rects: {} };
const tab = () => state.tabs?.find(t => t.id === state.active_tab), control = id => state.controls?.find(c => c.id === id);
const editor = () => tab()?.conflict_editor, ai = () => editor()?.ai;
async function until(fn, label = 'state', timeout = 45000) {
  const at = performance.now();
  for (;;) {
    state = await client.tool('rocket_state'); if (fn(state)) return;
    assert(performance.now() - at < timeout, label + ' timeout ' + JSON.stringify({ busy: state.busy, input: state.input_pending, dialog: state.dialog?.kind }));
    await delay(50);
  }
}
const settled = () => until(s => !s.busy && !s.input_pending && !(s.tabs || []).some(t => t.loading), 'settled');
async function stable() { let last = '', n = 0; await until(s => { const c = JSON.stringify(s.controls); n = c === last ? n + 1 : 0; last = c; return !s.input_pending && n >= 3; }, 'stable'); }
const onPage = id => id.startsWith('theme-') && Boolean(control('prefs-scroll'));
function shown(id) { const c = control(id), view = control('prefs-scroll')?.rect; return Boolean(c && (!onPage(id) || c.rect[1] > view[1] + 0.5 && c.rect[3] < view[3] - 0.5)); }
async function scroll(direction, pixels) { await client.tool('rocket_scroll', { target: 'prefs-scroll', direction, pixels, expected_tab: state.active_tab }); await settled(); await stable(); }
async function reveal(id) {
  await stable(); if (shown(id) || !onPage(id)) return;
  for (let i = 0; i < 3 && !shown(id); i++) await scroll('up', 2000);
  for (let i = 0; i < 25 && !shown(id); i++) await scroll('down', 200);
}
async function click(id, wait = true) {
  await reveal(id); assert(control(id)?.enabled, 'Missing or disabled ' + id);
  for (let i = 0; ; i++) {
    try { await client.tool('rocket_click', { target: id, expected_tab: state.active_tab }); break; }
    catch (e) { if (i < 40 && /settle/.test(e.message)) { await delay(250); await settled(); continue; } throw e; }
  }
  if (wait) await settled();
}
async function type(id, text) { await stable(); assert(control(id)?.enabled, 'Missing input ' + id); await client.tool('rocket_type', { target: id, text, expected_tab: state.active_tab }); await settled(); }
async function resize(w, h) { await stable(); await client.tool('rocket_resize', { width: w, height: h }); await until(s => s.viewport[0] === w && s.viewport[1] === h && !s.input_pending, 'resize'); await settled(); }

// Where each control sat in a shot, so the video's pointer lands on it.
const KEPT = /^(diff-line-\d+$|diff-hunk-|diff-apply-selection$|diff-split$|int-play-run$|int-play-ui-|palette-input$|palette-row-|commit-[0-9a-f]{7,}$|conflict-ai$|conflict-ai-apply$)/;
async function capture(name) {
  for (let tries = 0; ; tries++) {
    await stable(); await delay(250); await stable();
    assert.notEqual(state.dialog?.kind, 'quit', 'something outside this run closed the window');
    const r = await client.request('tools/call', { name: 'rocket_capture', arguments: {} }); assert(!r.isError);
    const png = Buffer.from(r.content.find(c => c.type === 'image').data, 'base64');
    const w = png.readUInt32BE(16), h = png.readUInt32BE(20);
    // The window's size can slip during a run, so each shot is checked.
    if ((w !== W || h !== H) && tries < 4) { await client.tool('rocket_resize', { width: W, height: H }); await delay(1500); continue; }
    fs.writeFileSync(path.join(out, name + '.png'), png);
    report.shots.push(name);
    report.rects[name] = Object.fromEntries((state.controls || []).filter(c => KEPT.test(c.id)).map(c => [c.id, c.rect]));
    console.error(`shot ${name} ${w}x${h}`); return;
  }
}
async function launch() {
  const log = fs.openSync(path.join(root, 'service.log'), 'a');
  service = cp.spawn(exe, ['--http-profile', profile], { env, windowsHide: true, stdio: ['ignore', log, log] }); fs.closeSync(log);
  client = new RocketHttpClient(profile);
  for (let i = 0; ; i++) { try { assert.equal((await client.raw('GET', '/health')).status, 200); break; } catch (e) { if (i >= 100) throw e; await delay(80); } }
  await client.initialize();
}
async function open(repo) {
  await client.tool('rocket_open', { path: repo }); await settled(); await resize(W, H);
  if (control('console-close')) await click('console-close');
}
async function theme(id) {
  await click('menu-view'); await click('view-appearance');
  await until(s => s.controls.some(c => c.id === 'view-theme-' + id), 'View > Appearance');
  await click('view-theme-' + id); if (control('view-appearance')) await click('menu-view');
  await until(s => s.appearance?.theme === id, 'theme ' + id); await settled();
}
// The site's pictures show Clay, the default theme: dark, and light for the light-mode copies. Clay's mode
// is set in Preferences, so each shot names its mode instead of following Windows.
async function look(mode) {
  state = await client.tool('rocket_state');
  if (state.appearance?.theme === 'clay' && state.appearance.palette === 'clay-' + mode) return;
  await prefs('pref-page-appearance'); await click('theme-clay'); await click('theme-clay-' + mode);
  await until(s => s.appearance?.theme === 'clay' && s.appearance.palette === 'clay-' + mode, 'Clay ' + mode);
  await click('dialog-close'); await settled();
}
async function prefs(page) {
  await click('menu-edit'); await click('preferences'); await until(s => s.dialog && control('pref-page-integrations'), 'Preferences'); await click(page);
}
// One failed step shouldn't cost the rest of the run.
async function attempt(name, fn) {
  if (only.size && !only.has(name)) return;
  try { await fn(); } catch (e) {
    report.failed.push({ name, error: String(e.message || e).slice(0, 400) }); console.error('FAILED ' + name + ': ' + e.message);
    try { await capture('failed-' + name); } catch {}
    try {
      state = await client.tool('rocket_state');
      if (control('dialog-cancel')) await click('dialog-cancel'); else if (control('dialog-close')) await click('dialog-close');
    } catch {}
  }
}
async function flipped(want) {
  const at = performance.now(); let nudged = false;
  for (;;) {
    state = await client.tool('rocket_state');
    if (tab()?.head?.branch === want && !tab().loading && !state.busy) return;
    if (!nudged && performance.now() - at > 5000) { nudged = true; await client.tool('rocket_shortcut', { binding: 'F5', expected_tab: state.active_tab }).catch(() => {}); }
    // A refresh after the flip takes 15-20 s on the bundled Git, and longer while other work runs.
    assert(performance.now() - at < 90000, 'branch did not become ' + want + ': ' + JSON.stringify(tab()?.head));
    await delay(100);
  }
}
async function shot(repo, real, name) {
  toReal(repo, real);
  try { await flipped(real); await settled(); await capture(name); }
  finally { toMark(repo); }
  await flipped(MARK); await settled();
}
const shotA = name => shot(repoA, 'feature/plane-change', name), shotB = name => shot(repoB, 'main', name);
const row = (file, kind = 'unstaged') => 'change-' + kind + '-' + Buffer.from(file).toString('hex');
const lineControls = () => state.controls.filter(c => /^diff-line-\d+$/.test(c.id)).sort((x, y) => Number(x.id.split('-').at(-1)) - Number(y.id.split('-').at(-1)));
async function changes() { await click('view-changes'); await click(row('src/transfer.rs')); }
// Every page shares one diff layout, so the Commits shots pick Unified instead of keeping the last step's.
async function commits() {
  await click('view-commits'); await click('history-scope'); await click('history-all');
  await until(s => s.controls.some(c => c.id === 'diff-unified'), 'Commits diff'); await click('diff-unified'); await settled();
}

// A real script for the Lua playground, so the picture shows what writing an extension looks like.
const LUA = [
  "-- A release checklist for this repository.",
  "return function(ctx)",
  "  local done = ctx.state.done or 1",
  "  if ctx.event == 'next' then done = math.min(done + 1, 4) end",
  "  local steps = { 'Run the tests', 'Update the changelog',",
  "                  'Tag the release', 'Publish the notes' }",
  "  local rows, keys = {}, {}",
  "  for i, step in ipairs(steps) do",
  "    rows[i] = { step, i <= done and { text = 'Done', tone = 'success' }",
  "                                 or { text = 'To do' } }",
  "    keys[i] = tostring(i)",
  "  end",
  "  return {",
  "    state = { done = done },",
  "    view = ui.page { title = 'Release checklist', children = {",
  "      ui.table { id = 'steps', columns = { 'Step', 'State' },",
  "                 keys = keys, rows = rows },",
  "      ui.progress { value = done / 4, label = done .. ' of 4 done' },",
  "      ui.button { id = 'next', label = 'Mark next done', primary = true },",
  "    } }",
  "  }",
  "end",
  ""
].join('\n');
async function playground() {
  await prefs('pref-page-integrations');
  if (!control('int-open-playground')) await click('int-developer');
  await click('int-open-playground');
  await until(s => s.integrations?.playground?.loaded && !s.integrations.playground.busy, 'playground');
  await type('int-play-source', LUA); await stable();
}
async function runLua() {
  await click('int-play-run'); await delay(400);
  await until(s => { const p = s.integrations?.playground; return p && !p.busy && (p.has_preview || p.error); }, 'preview', 20000);
  if (state.integrations.playground.error) throw Error('Lua: ' + state.integrations.playground.error);
  await stable();
}
async function reviewShot(name) {
  await click('int-play-back'); await settled();
  if (!control('int-enable-playground')) await prefs('pref-page-integrations');
  await click('int-enable-playground'); await until(s => s.integrations?.prompt?.kind === 'enable', 'review'); await stable(); await capture(name);
  await client.tool('rocket_escape'); await until(s => !s.integrations?.prompt, 'review closed', 8000); await settled();
  if (control('dialog-close')) await click('dialog-close');
}
async function conflictShots(suffix) {
  if (control('conflict-reload')) { await click('conflict-reload'); await click('dialog-submit'); await settled(); }
  else { await click('view-changes'); await click(row('src/transfer.rs')); await click('conflict-open'); }
  await shotB('conflict' + suffix);
  await click('conflict-ai'); await until(() => ai()?.open, 'AI page');
  if (ai().source !== 'agent') await click('conflict-ai-source-agent');
  await stable(); await click('conflict-ai-share', false); await until(() => ai()?.shared && !state.input_pending, 'shared');
  const id = ai().request_id, data = await client.tool('rocket_ai_conflict_get', { request_id: id });
  const answer = b => /abs/.test(b.ours + b.theirs)
    ? { replacement: '        (self.departure_burn.abs() + self.arrival_burn.abs() + self.plane_burn.abs()) * GRAVITY_LOSS_MARGIN\n', explanation: 'Keeps the gravity-loss margin and counts the new plane burn inside it.' }
    : { replacement: b.theirs.replace('coast: PI * (semi_major.powi(3) / mu).sqrt(),', 'coast: Orbit { radius: semi_major, mu }.period() / 2.0,'), explanation: 'Keeps the plane burn from this branch and the shorter coast from main.' };
  const proposal = JSON.stringify({ resolutions: data.context.sections.map(b => ({ id: b.id, ...answer(b), confidence: 'high' })) });
  await client.tool('rocket_ai_conflict_propose', { request_id: id, proposal });
  await until(() => ai()?.has_proposal && !ai().busy, 'proposal'); await shotB('proposal' + suffix);
  await click('conflict-ai-apply');
}

async function run() {
  build();
  cp.execFileSync(exe, ['--init-http', profile, root], { env, windowsHide: true });
  const listener = net.createServer(); await new Promise(r => listener.listen(0, '127.0.0.1', r));
  const port = listener.address().port; await new Promise(r => listener.close(r));
  fs.writeFileSync(path.join(profile, 'settings'), 'ROCKET-MCP-1\n' + port + '\n' + root + '\n');
  await launch();
  await open(repoA);

  await attempt('changes', async () => {
    await look('dark'); await changes(); await type('commit-message', 'Add plane changes to transfers');
    await click('diff-unified'); await shotA('changes-unified');
    await click('diff-split'); await shotA('changes-split');
  });
  await attempt('light', async () => {
    await look('light'); await changes();
    await click('diff-unified'); await shotA('changes-unified-light');
    await click('diff-split'); await shotA('changes-split-light');
    await commits(); await shotA('commits-clay-light');
  });
  // The Themes section's other built-in themes. Its two Clay shots come from the light and history steps.
  await attempt('themes', async () => { for (const id of ['ocean', 'dark', 'light']) { await theme(id); await commits(); await shotA('commits-' + id); } });
  await attempt('custom', async () => {
    await theme('ocean'); await prefs('pref-page-appearance');
    await click('theme-custom'); await click('theme-picture-astronaut'); await click('theme-accent-picture');
    await until(s => s.appearance?.custom?.picture_shown, 'picture'); await click('dialog-close');
    await commits(); await shotA('commits-custom');
  });
  await attempt('history', async () => {
    await look('dark'); await commits(); await shotA('commits-clay-dark');
    const id = tab().history_ids[3];
    await click('commit-' + id); await stable(); assert.equal(tab().selected_commit, id);
    await shotA('commit-selected'); assert.equal(tab().selected_commit, id);
  });
  await attempt('palette', async () => {
    await look('dark'); await click('view-changes');
    for (const text of ['r', 'reb', 'rebase']) {
      if (!control('palette-input')) {
        await client.tool('rocket_shortcut', { binding: 'Ctrl+K', expected_tab: state.active_tab });
        await until(s => s.controls.some(c => c.id === 'palette-input'), 'palette');
      }
      await type('palette-input', text); await shotA('palette-' + text); await settled();
    }
    await client.tool('rocket_escape'); await settled();
  });
  await attempt('lua', async () => {
    await look('dark'); await playground(); await shotA('lua-0');
    await runLua(); await shotA('lua-1');
    await click('int-play-ui-next'); await stable(); await shotA('lua-2');
    await click('int-play-ui-next'); await stable(); await shotA('lua-3');
    await reviewShot('review');
  });
  await attempt('lua-light', async () => {
    await look('light'); await playground(); await runLua();
    await click('int-play-ui-next'); await stable(); await shotA('lua-light');
    await reviewShot('review-light');
  });
  // Picking lines stays on the marked branch. The video shows only the diff pane, which names no branch.
  await attempt('stage', async () => {
    await look('dark'); await changes(); await click('diff-split'); await capture('stage-0');
    const lines = lineControls(), right = lines.filter(c => c.rect[0] > 1200), picks = right.length >= 3 ? right : lines;
    await click(picks[1].id); await capture('stage-1');
    await click(picks[2].id); await capture('stage-2');
    await click('diff-apply-selection'); await settled(); await capture('stage-3');
  });

  await client.tool('rocket_open', { path: repoB }); await settled(); if (control('console-close')) await click('console-close');
  for (const other of state.tabs.filter(t => t.id !== state.active_tab)) await click('close-tab-' + other.id);
  await attempt('conflict', async () => { await look('dark'); await conflictShots(''); });
  await attempt('conflict-light', async () => { await look('light'); await conflictShots('-light'); });

  await client.tool('rocket_open', { path: repoC }); await settled(); if (control('console-close')) await click('console-close');
  for (const [mode, suffix] of [['dark', ''], ['light', '-light']]) await attempt('rebase' + suffix, async () => {
    await look(mode);
    await click('menu-repository'); await click('repository-branch-menu'); await click('repository-interactive-rebase');
    await type('dialog-rebase-target', 'release/0.2'); await click('dialog-preview-refresh');
    await until(s => s.dialog?.rebase_preview_loaded || s.dialog?.error, 'rebase preview'); assert(state.dialog.rebase_preview_loaded); await stable();
    const act = async (i, name) => { await click('dialog-rebase-action-' + i); await click('dialog-rebase-action-' + i + '-' + name); };
    await act(2, 'fixup'); await act(4, 'drop'); await capture('rebase' + suffix);
    await click('dialog-cancel'); await settled();
  });
  await attempt('reset', () => look('dark'));
}

async function stop() {
  if (service && service.exitCode === null && service.signalCode === null) {
    try {
      if (state?.window === 'running') {
        await client.tool('rocket_close');
        await until(s => ['closed', 'not_started'].includes(s.window) || s.dialog?.kind === 'quit', 'closing', 15000);
        if (state.dialog?.kind === 'quit') { await client.tool('rocket_click', { target: 'dialog-submit', expected_tab: state.active_tab }); await until(s => ['closed', 'not_started'].includes(s.window), 'closed', 15000); }
      }
    } catch (e) { report.closeError = String(e.message || e).slice(0, 300); }
    await client.raw('POST', '/shutdown').catch(() => {});
    const at = performance.now();
    while (service.exitCode === null && service.signalCode === null) { if (performance.now() - at > 15000) { service.kill(); break; } await delay(50); }
  }
  if (client) await client.finish();
}

run().catch(e => { report.error = String(e.stack || e).slice(0, 1200); process.exitCode = 1; }).finally(async () => {
  await stop().catch(e => { report.stopError = String(e); });
  fs.writeFileSync(path.join(out, 'report.json'), JSON.stringify(report, null, 1));
  if (!report.failed.length && !report.error) fs.rmSync(root, { recursive: true, force: true, maxRetries: 3 });
  console.log(JSON.stringify({ out, shots: report.shots.length, failed: report.failed, error: report.error, closeError: report.closeError }, null, 1));
});
