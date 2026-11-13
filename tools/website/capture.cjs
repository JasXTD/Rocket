// Takes the website's screenshots from the real window through localhost MCP, in a throwaway demo
// repository with made-up authors. Nothing here pushes or touches a real repository.
// Claim the lock in COORDINATION.md first, and leave the Rocket window alone while it runs (about 10
// minutes). Usage: node tools/website/capture.cjs [output folder] [steps to retake], then
// node tools/website/media.cjs
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process'), os = require('node:os');
const net = require('node:net'), assert = require('node:assert/strict');
const { RocketHttpClient } = require('../mcp/mcp-http-client.cjs');
const project = path.resolve(__dirname, '../..');
// In the temp folder, not under work/: Agents refuses a worktree folder inside any repository, this
// checkout included. A failed run leaves the folder for a look; its path is in report.json.
const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'rocket-site-')));
const out = path.resolve(process.argv[2] || path.join(project, 'work/website-shots'));
const only = new Set(process.argv.slice(3));
const exe = path.join(project, 'target/release/rocket-mcp.exe');
const profile = path.join(root, 'http-profile');
const remote = path.join(root, 'remote', 'orbit-planner.git');
const repoA = path.join(root, 'a', 'orbit-planner'), repoB = path.join(root, 'b', 'orbit-planner'), repoC = path.join(root, 'c', 'orbit-planner');
const repoD = path.join(root, 'd', 'orbit-planner');
const marker = path.join(root, 'rocket-private-state'), MARK = 'rocket-ui-test/site';
const W = 1920, H = 1017;

for (const d of [out, path.join(root, 'empty-template'), repoA, repoB, path.dirname(repoC), path.dirname(remote), repoD]) fs.mkdirSync(d, { recursive: true });
for (const d of ['home', 'local', 'roaming']) fs.mkdirSync(path.join(root, d));
fs.writeFileSync(path.join(root, 'empty.config'), '');
fs.writeFileSync(marker, 'synthetic-content-only-v1\n');
const env = { ...process.env };
for (const k of Object.keys(env)) if (/^(GIT_|SSH_|GCM_|CODEX_|CLAUDE_|COPILOT_|OPENAI_|ANTHROPIC_)|_PROXY$/i.test(k)) delete env[k];
Object.assign(env, {
  GIT_CONFIG_GLOBAL: path.join(root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(root, 'empty.config'), GIT_CONFIG_NOSYSTEM: '1',
  GIT_ALLOW_PROTOCOL: 'file', GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0', GIT_TEMPLATE_DIR: path.join(root, 'empty-template'),
  USERPROFILE: path.join(root, 'home'), LOCALAPPDATA: path.join(root, 'local'), APPDATA: path.join(root, 'roaming'),
  // The Agents shots run rocket-mcp's own stand-in agent, which works only inside this folder.
  ROCKET_AGENT_FIXTURE_ROOT: root
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

// ---- A second demo project, for the Map and Insights: orbit-planner grown into a workspace of six
// Cargo packages, written by five made-up people over six months. git fast-import writes it, so the
// authors, dates and merges are exact. The Map reads only the newest files, so only those are whole:
// a file gains paragraphs as commits touch it, and each package's root file lists the modules present.
const TEAM = { // name, minutes from UTC, usual hours
  ada: ['Ada Example', 60, 9, 17], sam: ['Sam Example', -300, 10, 19], kai: ['Kai Example', 540, 12, 21],
  mia: ['Mia Example', 330, 8, 16], noor: ['Noor Example', 120, 9, 18]
};
const C = 'crates/orbit-core/', E = 'crates/orbit-ephemeris/', T = 'crates/orbit-transfer/', P = 'crates/orbit-plot/';
const L = 'crates/orbit-cli/', K = 'crates/orbit-testkit/';
const MANIFEST = (name, about, deps = [], dev = [], bin = '') => `[package]
name = "${name}"
description = "${about}"
version.workspace = true
edition.workspace = true
license.workspace = true
${bin}${deps.length ? '\n[dependencies]\n' + deps.map(d => `${d} = { path = "../${d}" }\n`).join('') : ''}${dev.length ? '\n[dev-dependencies]\n' + dev.map(d => `${d} = { path = "../${d}" }\n`).join('') : ''}`;
const WORKSPACE = version => `[workspace]
members = ["crates/*"]
resolver = "3"

[workspace.package]
version = "${version}"
edition = "2024"
license = "MIT OR Apache-2.0"
`;
const RELEASES = {
  '0.1.0': ['Circular and elliptical orbits around Earth and the Moon', "Kepler's equation and state vectors", 'Epochs, frames and angles that keep their unit'],
  '0.2.0': ['Hohmann and bi-elliptic transfers, with a margin for gravity losses', 'Plane changes', 'The Moon and the Sun from bundled tables', 'The plan command, with transfer and moon'],
  '0.3.0': ['Orbits and transfers as SVG drawings', 'Lambert arcs', 'A delta-v budget, and plan budget', 'The Moon table starts at J2000'],
  '0.4.0': ['Launch windows from phase angles', 'plan --json for scripts', 'A guide for new contributors']
};
const CHANGELOG = (versions, next) => '# Changelog\n' + (next ? '\n## Unreleased\n\n- Nothing yet\n' : '') +
  versions.map(v => `\n## ${v}\n\n` + RELEASES[v].map(line => '- ' + line + '\n').join('')).join('');
// The Moon from Earth every six hours, roughly: a tilted circle of 384,400 km.
const MOON_CSV = Array.from({ length: 48 }, (_, i) => {
  const a = Math.PI * 2 * i / 109.3, r = 384_400, tilt = 28.6 * Math.PI / 180;
  return [r * Math.cos(a), r * Math.sin(a) * Math.cos(tilt), r * Math.sin(a) * Math.sin(tilt)].map(v => v.toFixed(1)).join(',');
}).join('\n') + '\n';
const MAIN = `fn main() {
    let args = match args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}\\n\\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    if let Err(e) = commands::run(&args) {
        eprintln!("plan: {e}");
        std::process::exit(1);
    }
}
`;
const MAIN_FIRST = `fn main() {
    let args = args::parse(std::env::args().skip(1)).unwrap_or_else(|message| {
        eprintln!("{message}\\n\\n{}", args::USAGE);
        std::process::exit(2);
    });
    let from = orbit_core::Orbit::around_earth(args.from).expect("altitude");
    let to = orbit_core::Orbit::around_earth(args.to).expect("altitude");
    output::print_transfer(&orbit_transfer::hohmann(&from, &to).expect("one body"), false);
}
`;
// Each package's lib.rs or main.rs: its modules that exist at that commit, and what it re-exports.
const PKG = {
  'orbit-core': { about: 'Bodies, orbits and the units they are measured in.', mods: ['angle', 'body', 'consts', 'error', 'frame', 'kepler', 'orbit', 'state', 'time', 'units'],
    uses: { angle: 'angle::Angle', body: 'body::Body', error: 'error::{Error, Result}', orbit: 'orbit::Orbit', state: 'state::StateVector', time: 'time::Epoch' } },
  'orbit-ephemeris': { about: 'Where the Moon and the Sun are, from bundled tables.', mods: ['interpolate', 'moon', 'sun', 'table'], hidden: ['interpolate', 'table'],
    uses: { moon: 'moon::moon_position', sun: 'sun::sun_direction', table: 'table::Table' } },
  'orbit-transfer': { about: 'Burns between orbits: Hohmann, bi-elliptic, plane changes, Lambert arcs and launch windows.',
    mods: ['bi_elliptic', 'budget', 'hohmann', 'lambert', 'plane_change', 'window'], uses: { budget: 'budget::Budget', hohmann: 'hohmann::{hohmann, Transfer}' } },
  'orbit-plot': { about: 'Orbits and transfers as SVG drawings.', mods: ['scale', 'svg', 'track'], hidden: ['scale'], uses: { svg: 'svg::Drawing' } },
  'orbit-testkit': { about: 'Helpers that the tests of the other packages share.', mods: ['approx', 'fixtures'] },
  'orbit-cli': { about: 'plan: transfers between orbits, from the command line.', root: 'main.rs', mods: ['args', 'commands', 'output'], hidden: ['args', 'commands', 'output'],
    tail: has => has('commands') ? MAIN : MAIN_FIRST }
};
function crateRoot(name, has) {
  const p = PKG[name], present = p.mods.filter(has);
  const uses = present.filter(m => p.uses?.[m]).map(m => 'pub use ' + p.uses[m] + ';\n').join('');
  return '//! ' + p.about + '\n' + present.map(m => (p.hidden?.includes(m) ? '' : 'pub ') + 'mod ' + m + ';\n').join('') +
    (uses ? '\n' + uses : '') + (p.tail ? '\n' + p.tail(has) : '');
}
const FILES = {
  '.gitignore': '/target\n\n*.svg\n',
  'README.md': `# orbit-planner

Plan burns between orbits around one body: Hohmann and bi-elliptic transfers, plane changes, Lambert arcs and launch windows, with the Moon and the Sun from bundled tables.

Distances are in kilometres, speeds in km/s and times in seconds.

| Package | What it does |
| --- | --- |
| orbit-core | Bodies, orbits, angles, epochs and state vectors |
| orbit-ephemeris | Where the Moon and the Sun are |
| orbit-transfer | Burns between orbits, and a delta-v budget |
| orbit-plot | Orbits and transfers as SVG drawings |
| orbit-cli | The plan command |
| orbit-testkit | Fixtures the tests share |

    cargo run -p orbit-cli -- transfer 400 35786
    cargo run -p orbit-cli -- draw 400 35786 --svg transfer.svg

See docs/ for the guides, and docs/contributing.md before your first pull request.
`,
  '.github/workflows/ci.yml': `name: CI
on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace

      - uses: actions/cache@v4
        with:
          path: ~/.cargo/registry
          key: cargo-\${{ hashFiles('Cargo.lock') }}

      - run: cargo fmt --check
`,
  'docs/transfers.md': `# Transfers

A Hohmann transfer is half an ellipse that touches both orbits. It takes two burns: one to leave the first orbit and one to join the second.

    plan transfer 400 35786

When the outer orbit is more than about twelve times wider than the inner one, three burns through a higher orbit can cost less. orbit_transfer::bi_elliptic::worth_trying says when to try.

A launch window opens when the target leads by the right phase angle. orbit_transfer::window works out the angle and the next date it comes round.
`,
  'docs/ephemeris.md': `# The Moon and the Sun

orbit-ephemeris carries a table of the Moon's position every six hours and interpolates between rows with cubic Hermite curves. The Sun's direction comes from a short series, good to about a hundredth of a degree.
`,
  'docs/guide.md': `# Guide

Start with orbit-core for bodies and orbits, then add the packages you need.

    plan transfer 400 35786
    plan budget 400 35786
    plan draw 400 35786 --svg transfer.svg

The draw command writes an SVG with both orbits and the total speed change.
`,
  'docs/contributing.md': `# Contributing

Run cargo fmt and cargo test --workspace before you push. Keep each package's public items documented, and put shared test helpers in orbit-testkit.
`,
  [C + 'Cargo.toml']: MANIFEST('orbit-core', 'Bodies, orbits, angles, epochs and state vectors'),
  [C + 'src/consts.rs']: `//! Physical constants, in kilometres and seconds.

/// Gravitational parameter of Earth, km³/s².
pub const MU_EARTH: f64 = 398_600.4418;
/// Gravitational parameter of the Moon, km³/s².
pub const MU_MOON: f64 = 4_902.8;
/// Gravitational parameter of the Sun, km³/s².
pub const MU_SUN: f64 = 132_712_440_018.0;

/// Mean radius of Earth, km.
pub const RADIUS_EARTH: f64 = 6_371.0;
/// Mean radius of the Moon, km.
pub const RADIUS_MOON: f64 = 1_737.4;
/// One astronomical unit, km.
pub const AU: f64 = 149_597_870.7;
`,
  [C + 'src/units.rs']: `//! Conversions between the units people type and the ones the maths uses.

/// Seconds in so many hours.
pub fn hours(h: f64) -> f64 {
    h * 3_600.0
}

/// Seconds in so many days.
pub fn days(d: f64) -> f64 {
    d * 86_400.0
}

/// A speed in km/s, written for people.
pub fn format_speed(km_per_s: f64) -> String {
    format!("{km_per_s:.3} km/s")
}
`,
  [C + 'src/angle.rs']: `//! Angles that remember their unit.
use std::f64::consts::{PI, TAU};

/// An angle, kept in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Angle(f64);

impl Angle {
    pub fn degrees(d: f64) -> Self {
        Self(d.to_radians())
    }
    pub fn radians(r: f64) -> Self {
        Self(r)
    }
    pub fn as_radians(self) -> f64 {
        self.0
    }
    pub fn as_degrees(self) -> f64 {
        self.0.to_degrees()
    }

    /// The same angle, between -π and π.
    pub fn wrapped(self) -> Self {
        Self((self.0 + PI).rem_euclid(TAU) - PI)
    }
}
`,
  [C + 'src/error.rs']: `//! What can go wrong while planning.
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// An altitude below the surface of the body.
    BelowSurface { altitude: f64 },
    /// Two orbits around different bodies.
    DifferentBodies,
    /// An iteration that did not settle.
    NoConvergence { iterations: u32 },
    /// A date outside the loaded tables.
    OutOfRange,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::BelowSurface { altitude } => write!(f, "{altitude} km is below the surface"),
            Error::DifferentBodies => f.write_str("both orbits need the same body"),
            Error::NoConvergence { iterations } => write!(f, "no answer after {iterations} steps"),
            Error::OutOfRange => f.write_str("the date is outside the tables"),
        }
    }
}

impl std::error::Error for Error {}
`,
  [C + 'src/body.rs']: `//! The bodies an orbit can go around.
use crate::consts::{MU_EARTH, MU_MOON, RADIUS_EARTH, RADIUS_MOON};

/// A central body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub name: &'static str,
    /// Gravitational parameter, km³/s².
    pub mu: f64,
    /// Mean radius, km.
    pub radius: f64,
}

pub const EARTH: Body = Body { name: "Earth", mu: MU_EARTH, radius: RADIUS_EARTH };
pub const MOON: Body = Body { name: "Moon", mu: MU_MOON, radius: RADIUS_MOON };

impl Body {
    /// Speed of a circular orbit so many km from the centre, km/s.
    pub fn circular_speed(&self, radius: f64) -> f64 {
        (self.mu / radius).sqrt()
    }

    /// Speed needed to escape from so many km, km/s.
    pub fn escape_speed(&self, radius: f64) -> f64 {
        (2.0 * self.mu / radius).sqrt()
    }
}
`,
  [C + 'src/orbit.rs']: `//! Circular and elliptical orbits around one body.
use crate::angle::Angle;
use crate::body::{Body, EARTH};
use crate::error::{Error, Result};
use std::f64::consts::TAU;

/// An orbit around one body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub body: Body,
    /// Closest distance from the centre of the body, km.
    pub periapsis: f64,
    /// Farthest distance from the centre of the body, km.
    pub apoapsis: f64,
    /// Tilt against the body's equator.
    pub inclination: Angle,
}

impl Orbit {
    /// A circular orbit so many km above the surface.
    pub fn circular(body: Body, altitude: f64) -> Result<Self> {
        if altitude < 0.0 {
            return Err(Error::BelowSurface { altitude });
        }
        let r = body.radius + altitude;
        Ok(Self { body, periapsis: r, apoapsis: r, inclination: Angle::default() })
    }

    /// A circular orbit around Earth.
    pub fn around_earth(altitude: f64) -> Result<Self> {
        Self::circular(EARTH, altitude)
    }

    pub fn semi_major_axis(&self) -> f64 {
        (self.periapsis + self.apoapsis) / 2.0
    }

    pub fn eccentricity(&self) -> f64 {
        (self.apoapsis - self.periapsis) / (self.apoapsis + self.periapsis)
    }

    /// Speed so many km from the centre, from the vis-viva equation, km/s.
    pub fn speed_at(&self, radius: f64) -> f64 {
        (self.body.mu * (2.0 / radius - 1.0 / self.semi_major_axis())).sqrt()
    }

    /// Time for one orbit, seconds.
    pub fn period(&self) -> f64 {
        TAU * (self.semi_major_axis().powi(3) / self.body.mu).sqrt()
    }

    /// The same orbit, turned to another inclination.
    pub fn inclined(self, inclination: Angle) -> Self {
        Self { inclination, ..self }
    }
}
`,
  [C + 'src/kepler.rs']: `//! Kepler's equation: where a craft is along its orbit at a given time.
use crate::angle::Angle;
use crate::error::{Error, Result};

const TOLERANCE: f64 = 1e-12;
const MAX_STEPS: u32 = 50;

/// Eccentric anomaly for a mean anomaly, by Newton's method.
pub fn eccentric_anomaly(mean: Angle, eccentricity: f64) -> Result<Angle> {
    let m = mean.wrapped().as_radians();
    let mut e = if eccentricity < 0.8 { m } else { std::f64::consts::PI };
    for _ in 0..MAX_STEPS {
        let step = (e - eccentricity * e.sin() - m) / (1.0 - eccentricity * e.cos());
        e -= step;
        if step.abs() < TOLERANCE {
            return Ok(Angle::radians(e));
        }
    }
    Err(Error::NoConvergence { iterations: MAX_STEPS })
}

/// True anomaly from the eccentric anomaly.
pub fn true_anomaly(eccentric: Angle, eccentricity: f64) -> Angle {
    let half = eccentric.as_radians() / 2.0;
    let factor = ((1.0 + eccentricity) / (1.0 - eccentricity)).sqrt();
    Angle::radians(2.0 * (factor * half.tan()).atan())
}
`,
  [C + 'src/time.rs']: `//! Epochs, as seconds since J2000.
use crate::error::{Error, Result};
use crate::units::days;

/// Seconds since 2000-01-01 12:00 TT.
#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
pub struct Epoch(pub f64);

impl Epoch {
    pub const J2000: Epoch = Epoch(0.0);

    /// An epoch from a Julian date.
    pub fn from_julian(date: f64) -> Result<Self> {
        if !date.is_finite() {
            return Err(Error::OutOfRange);
        }
        Ok(Self(days(date - 2_451_545.0)))
    }

    pub fn plus(self, seconds: f64) -> Self {
        Self(self.0 + seconds)
    }

    pub fn days_since(self, other: Epoch) -> f64 {
        (self.0 - other.0) / 86_400.0
    }
}
`,
  [C + 'src/frame.rs']: `//! Reference frames for positions.
use crate::angle::Angle;

/// Earth's tilt against the ecliptic at J2000, degrees.
pub const OBLIQUITY: f64 = 23.439_291;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Centred on the body, lined up with its equator.
    Equatorial,
    /// Lined up with Earth's orbit around the Sun.
    Ecliptic,
}

/// An equatorial position in the ecliptic frame.
pub fn to_ecliptic([x, y, z]: [f64; 3]) -> [f64; 3] {
    let (s, c) = Angle::degrees(OBLIQUITY).as_radians().sin_cos();
    [x, c * y + s * z, -s * y + c * z]
}
`,
  [C + 'src/state.rs']: `//! Position and velocity at one moment.
use crate::frame::Frame;
use crate::kepler::{eccentric_anomaly, true_anomaly};
use crate::orbit::Orbit;
use crate::time::Epoch;
use crate::{Angle, Result};
use std::f64::consts::TAU;

/// Where a craft is and how fast it moves, km and km/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateVector {
    pub epoch: Epoch,
    pub frame: Frame,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
}

impl StateVector {
    /// A craft on an orbit, so many seconds after periapsis, in the orbit's plane.
    pub fn on_orbit(orbit: &Orbit, epoch: Epoch, elapsed: f64) -> Result<Self> {
        let e = orbit.eccentricity();
        let mean = Angle::radians(TAU * elapsed / orbit.period());
        let nu = true_anomaly(eccentric_anomaly(mean, e)?, e).as_radians();
        let p = orbit.semi_major_axis() * (1.0 - e * e);
        let (r, k) = (p / (1.0 + e * nu.cos()), (orbit.body.mu / p).sqrt());
        Ok(Self {
            epoch: epoch.plus(elapsed),
            frame: Frame::Equatorial,
            position: [r * nu.cos(), r * nu.sin(), 0.0],
            velocity: [-k * nu.sin(), k * (e + nu.cos()), 0.0],
        })
    }

    /// Distance from the centre of the body, km.
    pub fn radius(&self) -> f64 {
        self.position.iter().map(|v| v * v).sum::<f64>().sqrt()
    }
}
`,
  [C + 'tests/kepler.rs']: `use orbit_core::kepler::{eccentric_anomaly, true_anomaly};
use orbit_core::Angle;

#[test]
fn a_worked_example() {
    let e = eccentric_anomaly(Angle::radians(3.6029), 0.37255).unwrap();
    assert!((e.as_radians() - 3.4794).abs() < 1e-4);
    let nu = true_anomaly(e, 0.37255).wrapped();
    assert!((nu.as_degrees() + 166.2).abs() < 0.5);
}
`,
  [K + 'Cargo.toml']: MANIFEST('orbit-testkit', 'Fixtures and comparisons the tests share', ['orbit-core']),
  [K + 'src/approx.rs']: `//! Comparing floating-point results.

/// Panics unless the actual value is within the tolerance of the expected one.
#[track_caller]
pub fn assert_close(actual: f64, expected: f64, tolerance: f64) {
    assert!((actual - expected).abs() <= tolerance, "{actual} is not within {tolerance} of {expected}");
}
`,
  [K + 'src/fixtures.rs']: `//! Orbits that tests use again and again.
use orbit_core::Orbit;

/// Roughly the International Space Station's orbit.
pub fn low_earth() -> Orbit {
    Orbit::around_earth(420.0).unwrap()
}

/// A geostationary orbit.
pub fn geostationary() -> Orbit {
    Orbit::around_earth(35_786.0).unwrap()
}
`,
  [E + 'Cargo.toml']: MANIFEST('orbit-ephemeris', 'Where the Moon and the Sun are', ['orbit-core']),
  [E + 'data/moon.csv']: MOON_CSV,
  [E + 'src/table.rs']: `//! Positions sampled at fixed steps.
use orbit_core::{Epoch, Error, Result};

/// One body's position, one row every so many seconds from the start.
pub struct Table {
    pub start: Epoch,
    pub step: f64,
    pub rows: Vec<[f64; 3]>,
}

impl Table {
    /// The row just before an epoch, and how far towards the next one it lies.
    pub fn around(&self, epoch: Epoch) -> Result<(usize, f64)> {
        let at = (epoch.0 - self.start.0) / self.step;
        if at < 0.0 || at + 1.0 >= self.rows.len() as f64 {
            return Err(Error::OutOfRange);
        }
        Ok((at as usize, at.fract()))
    }
}
`,
  [E + 'src/interpolate.rs']: `//! Smooth positions between table rows.

use crate::table::Table;
use orbit_core::{Epoch, Result};

/// The position at an epoch, on a cubic Hermite curve through the nearest rows.
pub fn position(table: &Table, epoch: Epoch) -> Result<[f64; 3]> {
    let (i, t) = table.around(epoch)?;
    let (a, b) = (table.rows[i], table.rows[i + 1]);
    let before = if i > 0 { table.rows[i - 1] } else { a };
    let after = table.rows.get(i + 2).copied().unwrap_or(b);
    let (t2, t3) = (t * t, t * t * t);
    Ok([0, 1, 2].map(|k| {
        let (m0, m1) = ((b[k] - before[k]) / 2.0, (after[k] - a[k]) / 2.0);
        (2.0 * t3 - 3.0 * t2 + 1.0) * a[k] + (t3 - 2.0 * t2 + t) * m0 + (3.0 * t2 - 2.0 * t3) * b[k] + (t3 - t2) * m1
    }))
}
`,
  [E + 'src/moon.rs']: `//! The Moon's position around Earth.
use crate::interpolate::position;
use crate::table::Table;
use orbit_core::{Epoch, Result};
use std::sync::OnceLock;

static TABLE: OnceLock<Table> = OnceLock::new();

/// Where the Moon is from Earth's centre, km, in Earth's equatorial frame.
pub fn moon_position(epoch: Epoch) -> Result<[f64; 3]> {
    position(TABLE.get_or_init(load), epoch)
}

fn load() -> Table {
    let rows = include_str!("../data/moon.csv").lines().filter_map(|line| {
        let mut v = line.split(',').map(|x| x.trim().parse::<f64>().ok());
        Some([v.next()??, v.next()??, v.next()??])
    });
    Table { start: Epoch::J2000, step: 21_600.0, rows: rows.collect() }
}
`,
  [E + 'src/sun.rs']: `//! The direction of the Sun, from a short series.
use orbit_core::frame::OBLIQUITY;
use orbit_core::{Angle, Epoch};

/// A unit vector from Earth towards the Sun, in Earth's equatorial frame.
pub fn sun_direction(epoch: Epoch) -> [f64; 3] {
    let d = epoch.days_since(Epoch::J2000);
    let g = Angle::degrees(357.529 + 0.985_600_28 * d).as_radians();
    let longitude = Angle::degrees(280.459 + 0.985_647_36 * d + 1.915 * g.sin() + 0.020 * (2.0 * g).sin());
    let (sl, cl) = longitude.as_radians().sin_cos();
    let (se, ce) = OBLIQUITY.to_radians().sin_cos();
    [cl, ce * sl, se * sl]
}
`,
  [T + 'Cargo.toml']: MANIFEST('orbit-transfer', 'Burns between orbits, and a delta-v budget', ['orbit-core', 'orbit-ephemeris'], ['orbit-testkit']),
  [T + 'src/hohmann.rs']: `//! Half an ellipse that touches both orbits.
use orbit_core::{Error, Orbit, Result};

/// Margin for speed lost to gravity during a burn that takes minutes.
const GRAVITY_LOSS_MARGIN: f64 = 1.02;

/// The two burns of a transfer and the coast between them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transfer {
    /// Speed change at departure, km/s.
    pub departure: f64,
    /// Speed change at arrival, km/s.
    pub arrival: f64,
    /// Coast between the burns, seconds.
    pub coast: f64,
}

impl Transfer {
    /// Total speed change, km/s, with a margin for gravity losses.
    pub fn total(&self) -> f64 {
        (self.departure.abs() + self.arrival.abs()) * GRAVITY_LOSS_MARGIN
    }
}

/// A Hohmann transfer between two circular orbits around the same body.
pub fn hohmann(from: &Orbit, to: &Orbit) -> Result<Transfer> {
    if from.body != to.body {
        return Err(Error::DifferentBodies);
    }
    let (r1, r2) = (from.periapsis, to.periapsis);
    let ellipse = Orbit { periapsis: r1.min(r2), apoapsis: r1.max(r2), ..*from };
    Ok(Transfer {
        departure: ellipse.speed_at(r1) - from.speed_at(r1),
        arrival: to.speed_at(r2) - ellipse.speed_at(r2),
        coast: ellipse.period() / 2.0,
    })
}
`,
  [T + 'src/plane_change.rs']: `//! Turning the plane of an orbit.
use orbit_core::{Angle, Orbit};

/// The burn that turns an orbit by an angle at the same speed, km/s.
pub fn plane_change(orbit: &Orbit, angle: Angle) -> f64 {
    2.0 * orbit.speed_at(orbit.apoapsis) * (angle.as_radians() / 2.0).sin()
}

/// The cheapest place to turn: the slowest point, the apoapsis.
pub fn best_radius(orbit: &Orbit) -> f64 {
    orbit.apoapsis
}
`,
  [T + 'src/bi_elliptic.rs']: `//! Three burns through a higher orbit, cheaper than Hohmann for very different orbits.
use crate::hohmann::{hohmann, Transfer};
use orbit_core::{Orbit, Result};

/// A bi-elliptic transfer that turns round so many km from the centre.
pub fn bi_elliptic(from: &Orbit, to: &Orbit, turn: f64) -> Result<Transfer> {
    let high = Orbit { periapsis: turn, apoapsis: turn, ..*from };
    let (up, down) = (hohmann(from, &high)?, hohmann(&high, to)?);
    Ok(Transfer {
        departure: up.departure,
        arrival: up.arrival.abs() + down.departure.abs() + down.arrival.abs(),
        coast: up.coast + down.coast,
    })
}

/// Whether a bi-elliptic transfer can beat Hohmann between these orbits.
pub fn worth_trying(from: &Orbit, to: &Orbit) -> bool {
    to.periapsis / from.periapsis > 11.94
}
`,
  [T + 'src/lambert.rs']: `//! Lambert's problem: the arc between two positions in a given time.
use orbit_core::{Body, Error, Result};
use std::f64::consts::PI;

/// Velocities at both ends of an arc, km/s.
pub struct Arc {
    pub depart: [f64; 3],
    pub arrive: [f64; 3],
}

/// The arc from r1 to r2 that takes so many seconds, with universal variables.
pub fn lambert(body: &Body, r1: [f64; 3], r2: [f64; 3], seconds: f64) -> Result<Arc> {
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let (n1, n2) = (norm(r1), norm(r2));
    let cos = (r1[0] * r2[0] + r1[1] * r2[1] + r1[2] * r2[2]) / (n1 * n2);
    let a = (n1 * n2 * (1.0 + cos)).sqrt();
    let y = |z: f64| {
        let (c, s) = stumpff(z);
        n1 + n2 + a * (z * s - 1.0) / c.sqrt()
    };
    let time = |z: f64| {
        let (c, s) = stumpff(z);
        ((y(z) / c).powf(1.5) * s + a * y(z).sqrt()) / body.mu.sqrt()
    };
    let (mut low, mut high) = (-4.0 * PI * PI, 4.0 * PI * PI);
    for _ in 0..200 {
        let z = (low + high) / 2.0;
        if (time(z) - seconds).abs() < 1e-6 {
            let (f, g, gdot) = (1.0 - y(z) / n1, a * (y(z) / body.mu).sqrt(), 1.0 - y(z) / n2);
            return Ok(Arc {
                depart: [0, 1, 2].map(|k| (r2[k] - f * r1[k]) / g),
                arrive: [0, 1, 2].map(|k| (gdot * r2[k] - r1[k]) / g),
            });
        }
        if time(z) < seconds { low = z } else { high = z }
    }
    Err(Error::NoConvergence { iterations: 200 })
}

/// The Stumpff functions C(z) and S(z).
fn stumpff(z: f64) -> (f64, f64) {
    if z > 1e-6 {
        let s = z.sqrt();
        ((1.0 - s.cos()) / z, (s - s.sin()) / s.powi(3))
    } else if z < -1e-6 {
        let s = (-z).sqrt();
        ((s.cosh() - 1.0) / -z, (s.sinh() - s) / s.powi(3))
    } else {
        (0.5, 1.0 / 6.0)
    }
}
`,
  [T + 'src/budget.rs']: `//! Adding up the burns of a mission.
use crate::hohmann::Transfer;
use orbit_core::units::format_speed;

/// The burns of a mission, in order.
#[derive(Default)]
pub struct Budget {
    steps: Vec<(String, f64)>,
}

impl Budget {
    pub fn add(&mut self, name: impl Into<String>, delta_v: f64) {
        self.steps.push((name.into(), delta_v.abs()));
    }

    pub fn add_transfer(&mut self, name: &str, transfer: &Transfer) {
        self.add(format!("{name}: departure"), transfer.departure);
        self.add(format!("{name}: arrival"), transfer.arrival);
    }

    pub fn total(&self) -> f64 {
        self.steps.iter().map(|(_, v)| v).sum()
    }

    pub fn lines(&self) -> impl Iterator<Item = String> + '_ {
        self.steps.iter().map(|(name, v)| format!("{name:<28} {}", format_speed(*v)))
    }
}
`,
  [T + 'src/window.rs']: `//! Launch windows: when the target is in the right place for a transfer.
use crate::hohmann::hohmann;
use orbit_core::{Epoch, Orbit, Result};
use orbit_ephemeris::sun_direction;
use std::f64::consts::PI;

/// How far the target must lead at departure, radians.
pub fn phase_angle(from: &Orbit, to: &Orbit) -> Result<f64> {
    let coast = hohmann(from, to)?.coast;
    Ok(PI - coast * (to.body.mu / to.periapsis.powi(3)).sqrt())
}

/// The first day after an epoch with the Sun at least so far round from the reference direction.
pub fn next_window(after: Epoch, angle: f64) -> Epoch {
    let mut at = after;
    while sun_direction(at)[1].atan2(sun_direction(at)[0]) < angle {
        at = at.plus(86_400.0);
    }
    at
}
`,
  [T + 'tests/hohmann.rs']: `use orbit_testkit::approx::assert_close;
use orbit_testkit::fixtures::{geostationary, low_earth};
use orbit_transfer::hohmann;

#[test]
fn low_orbit_to_geostationary() {
    let transfer = hohmann(&low_earth(), &geostationary()).unwrap();
    assert_close(transfer.departure, 2.42, 0.01);
    assert_close(transfer.arrival, 1.46, 0.01);
}
`,
  [T + 'tests/lambert.rs']: `use orbit_core::body::EARTH;
use orbit_testkit::approx::assert_close;
use orbit_transfer::lambert::lambert;

#[test]
fn a_quarter_orbit_arc_follows_the_circle() {
    let r = 7_000.0;
    let period = std::f64::consts::TAU * (r * r * r / EARTH.mu).sqrt();
    let arc = lambert(&EARTH, [r, 0.0, 0.0], [0.0, r, 0.0], period / 4.0).unwrap();
    assert_close(arc.depart[1], (EARTH.mu / r).sqrt(), 1e-3);
}
`,
  [T + 'examples/leo_to_geo.rs']: `//! cargo run -p orbit-transfer --example leo_to_geo
use orbit_core::Orbit;
use orbit_transfer::{hohmann, Budget};

fn main() -> orbit_core::Result<()> {
    let transfer = hohmann(&Orbit::around_earth(400.0)?, &Orbit::around_earth(35_786.0)?)?;
    let mut budget = Budget::default();
    budget.add_transfer("LEO to GEO", &transfer);
    budget.lines().for_each(|line| println!("{line}"));
    Ok(())
}
`,
  [P + 'Cargo.toml']: MANIFEST('orbit-plot', 'Orbits and transfers as SVG drawings', ['orbit-core', 'orbit-transfer']),
  [P + 'src/scale.rs']: `//! Fitting kilometres into a picture.

/// A square so many km wide around the centre, drawn so many pixels wide.
#[derive(Clone, Copy)]
pub struct Scale {
    pub extent: f64,
    pub size: f64,
}

impl Scale {
    pub fn fit(largest_radius: f64, size: f64) -> Self {
        Self { extent: largest_radius * 2.2, size }
    }

    pub fn point(&self, [x, y]: [f64; 2]) -> (f64, f64) {
        let k = self.size / self.extent;
        (self.size / 2.0 + x * k, self.size / 2.0 - y * k)
    }
}
`,
  [P + 'src/track.rs']: `//! Points along an orbit, for drawing.
use orbit_core::{Epoch, Orbit, Result, StateVector};

/// So many positions spread over one period, in the orbit's plane.
pub fn track(orbit: &Orbit, count: usize) -> Result<Vec<[f64; 2]>> {
    let step = orbit.period() / count as f64;
    (0..=count)
        .map(|i| StateVector::on_orbit(orbit, Epoch::J2000, step * i as f64).map(|s| [s.position[0], s.position[1]]))
        .collect()
}
`,
  [P + 'src/svg.rs']: `//! A small SVG writer.
use crate::scale::Scale;
use crate::track::track;
use orbit_core::{Orbit, Result};
use orbit_transfer::Transfer;
use std::fmt::Write;

/// An SVG picture being built.
pub struct Drawing {
    scale: Scale,
    body: String,
}

impl Drawing {
    pub fn new(largest_radius: f64, size: f64) -> Self {
        Self { scale: Scale::fit(largest_radius, size), body: String::new() }
    }

    /// Adds an orbit as a closed line.
    pub fn orbit(&mut self, orbit: &Orbit, colour: &str) -> Result<&mut Self> {
        let points: Vec<String> = track(orbit, 180)?
            .into_iter()
            .map(|p| self.scale.point(p))
            .map(|(x, y)| format!("{x:.1},{y:.1}"))
            .collect();
        let _ = writeln!(self.body, r#"<polyline points="{}" fill="none" stroke="{colour}"/>"#, points.join(" "));
        Ok(self)
    }

    /// Adds the total of a transfer as a caption.
    pub fn caption(&mut self, transfer: &Transfer) -> &mut Self {
        let _ = writeln!(self.body, r#"<text x="12" y="24">Δv {:.2} km/s</text>"#, transfer.total());
        self
    }

    pub fn finish(&self) -> String {
        let s = self.scale.size;
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {s} {s}">{}</svg>"#, self.body)
    }
}
`,
  [L + 'Cargo.toml']: MANIFEST('orbit-cli', 'The plan command', ['orbit-core', 'orbit-ephemeris', 'orbit-transfer', 'orbit-plot'], [], '\n[[bin]]\nname = "plan"\npath = "src/main.rs"\n'),
  [L + 'src/args.rs']: `//! Command-line arguments.

pub const USAGE: &str = "usage: plan <transfer|budget|moon|draw> <from-km> <to-km> [--json] [--svg FILE]";

/// What to do.
pub enum Command {
    Transfer,
    Budget,
    Moon,
    Draw,
}

/// Everything the command line says.
pub struct Args {
    pub command: Command,
    pub from: f64,
    pub to: f64,
    pub json: bool,
    pub svg: Option<String>,
}

pub fn parse(mut words: impl Iterator<Item = String>) -> Result<Args, String> {
    let command = match words.next().as_deref() {
        Some("transfer") => Command::Transfer,
        Some("budget") => Command::Budget,
        Some("moon") => Command::Moon,
        Some("draw") => Command::Draw,
        other => return Err(format!("unknown command {other:?}")),
    };
    let (mut numbers, mut json, mut svg) = (Vec::new(), false, None);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--json" => json = true,
            "--svg" => svg = words.next(),
            n => numbers.push(n.parse::<f64>().map_err(|_| format!("not a number: {n}"))?),
        }
    }
    let [from, to] = numbers[..] else {
        return Err("give two altitudes in km".into());
    };
    Ok(Args { command, from, to, json, svg })
}
`,
  [L + 'src/output.rs']: `//! Printing results for people, or for scripts.

use orbit_core::units::format_speed;
use orbit_transfer::{Budget, Transfer};

pub fn print_transfer(t: &Transfer, json: bool) {
    if json {
        println!(r#"{{"departure":{:.4},"arrival":{:.4},"total":{:.4},"coast":{:.0}}}"#, t.departure, t.arrival, t.total(), t.coast);
        return;
    }
    println!("departure  {}", format_speed(t.departure));
    println!("arrival    {}", format_speed(t.arrival));
    println!("total      {}", format_speed(t.total()));
    println!("coast      {:.0} min", t.coast / 60.0);
}

pub fn print_budget(budget: &Budget) {
    for line in budget.lines() {
        println!("{line}");
    }
    println!("{:<28} {}", "total", format_speed(budget.total()));
}
`,
  [L + 'src/commands.rs']: `//! What each command does.
use crate::args::{Args, Command};
use crate::output::{print_budget, print_transfer};
use orbit_core::{Epoch, Orbit};
use orbit_ephemeris::moon_position;
use orbit_plot::Drawing;
use orbit_transfer::{hohmann, Budget, Transfer};

type Outcome = Result<(), Box<dyn std::error::Error>>;

pub fn run(args: &Args) -> Outcome {
    let (from, to) = (Orbit::around_earth(args.from)?, Orbit::around_earth(args.to)?);
    let transfer = hohmann(&from, &to)?;
    match args.command {
        Command::Transfer => print_transfer(&transfer, args.json),
        Command::Budget => budget(&transfer),
        Command::Moon => moon()?,
        Command::Draw => draw(&from, &to, &transfer, args.svg.as_deref())?,
    }
    Ok(())
}

fn budget(transfer: &Transfer) {
    let mut budget = Budget::default();
    budget.add_transfer("Hohmann", transfer);
    print_budget(&budget);
}

fn moon() -> Outcome {
    let [x, y, z] = moon_position(Epoch::J2000)?;
    println!("Moon at J2000: {x:.0} {y:.0} {z:.0} km");
    Ok(())
}

fn draw(from: &Orbit, to: &Orbit, transfer: &Transfer, file: Option<&str>) -> Outcome {
    let mut drawing = Drawing::new(to.apoapsis, 640.0);
    drawing.orbit(from, "#3b82f6")?.orbit(to, "#f59e0b")?.caption(transfer);
    std::fs::write(file.unwrap_or("transfer.svg"), drawing.finish())?;
    Ok(())
}
`
};
// Who did what, in order. A branch starts from the branch in use; merge goes back to main.
const WS_PLAN = [
  ['ada', 'Start orbit-planner as a Cargo workspace', ['.gitignore', 'README.md', C + 'Cargo.toml', C + 'src/consts.rs', C + 'src/body.rs', C + 'src/orbit.rs']],
  ['ada', 'Add angles that remember their unit', [C + 'src/angle.rs', C + 'src/orbit.rs']],
  ['sam', 'Add an error type', [C + 'src/error.rs']],
  ['ada', 'Return an error for altitudes below the surface', [C + 'src/orbit.rs']],
  ['noor', 'Run the tests on every push', ['.github/workflows/ci.yml']],
  ['ada', 'Add unit conversions', [C + 'src/units.rs', C + 'src/body.rs']],
  ['ada', 'Say which units the README uses', ['README.md']],
  ['branch', 'feature/kepler'],
  ['ada', "Solve Kepler's equation", [C + 'src/kepler.rs']],
  ['ada', 'Start from pi for eccentric orbits', [C + 'src/kepler.rs']],
  ['sam', 'Test Kepler against a worked example', [C + 'tests/kepler.rs']],
  ['ada', 'Add the true anomaly', [C + 'src/kepler.rs']],
  ['merge', 'feature/kepler', 'ada'],
  ['mia', 'Add epochs since J2000', [C + 'src/time.rs']],
  ['mia', 'Add reference frames', [C + 'src/frame.rs']],
  ['ada', 'Add state vectors', [C + 'src/state.rs']],
  ['sam', 'Add orbit-testkit with shared fixtures', [K + 'Cargo.toml', K + 'src/approx.rs', K + 'src/fixtures.rs']],
  ['ada', 'Add the escape speed', [C + 'src/body.rs']],
  ['release', 'ada', '0.1.0'],
  ['branch', 'feature/transfers'],
  ['sam', 'Add orbit-transfer with Hohmann transfers', [T + 'Cargo.toml', T + 'src/hohmann.rs']],
  ['sam', 'Test low orbit to geostationary', [T + 'tests/hohmann.rs']],
  ['sam', 'Add a margin for gravity losses', [T + 'src/hohmann.rs']],
  ['merge', 'feature/transfers', 'ada'],
  ['sam', 'Add plane changes', [T + 'src/plane_change.rs']],
  ['kai', 'Add the plan command', [L + 'Cargo.toml', L + 'src/args.rs', L + 'src/output.rs']],
  ['kai', 'Print speeds with their unit', [L + 'src/output.rs', C + 'src/units.rs']],
  ['ada', 'Explain Hohmann transfers in the guide', ['docs/transfers.md']],
  ['branch', 'feature/moon-ephemeris'],
  ['mia', "Add orbit-ephemeris with the Moon's table", [E + 'Cargo.toml', E + 'src/table.rs', E + 'src/moon.rs', E + 'data/moon.csv']],
  ['mia', 'Interpolate between table rows', [E + 'src/interpolate.rs']],
  ['mia', 'Use cubic Hermite curves between rows', [E + 'src/interpolate.rs']],
  ['mia', "Add the Sun's direction", [E + 'src/sun.rs']],
  ['mia', 'Describe the tables and their accuracy', ['docs/ephemeris.md']],
  ['merge', 'feature/moon-ephemeris', 'ada'],
  ['kai', 'Move each command into its own function', [L + 'src/commands.rs', L + 'src/args.rs']],
  ['kai', 'Add the moon command', [L + 'src/commands.rs']],
  ['sam', 'Add bi-elliptic transfers', [T + 'src/bi_elliptic.rs']],
  ['sam', 'Say when bi-elliptic is worth trying', [T + 'src/bi_elliptic.rs', 'docs/transfers.md']],
  ['noor', 'Cache the Cargo registry in CI', ['.github/workflows/ci.yml']],
  ['ada', 'List the packages in the README', ['README.md']],
  ['release', 'ada', '0.2.0'],
  ['branch', 'feature/svg-plots'],
  ['noor', 'Add orbit-plot to draw orbits as SVG', [P + 'Cargo.toml', P + 'src/svg.rs', P + 'src/scale.rs']],
  ['noor', 'Draw tracks from state vectors', [P + 'src/track.rs', P + 'src/svg.rs']],
  ['noor', 'Label a drawing with its total', [P + 'src/svg.rs']],
  ['merge', 'feature/svg-plots', 'noor'],
  ['kai', 'Add the draw command', [L + 'src/commands.rs', L + 'src/args.rs']],
  ['ada', 'Show the commands in the guide', ['docs/guide.md']],
  ['branch', 'feature/lambert'],
  ['sam', 'Add a Lambert solver', [T + 'src/lambert.rs']],
  ['sam', 'Bisect on z instead of stepping', [T + 'src/lambert.rs']],
  ['sam', 'Test a quarter-orbit arc', [T + 'tests/lambert.rs']],
  ['ada', 'Use Stumpff functions for hyperbolic arcs', [T + 'src/lambert.rs']],
  ['merge', 'feature/lambert', 'sam'],
  ['sam', 'Add a delta-v budget', [T + 'src/budget.rs']],
  ['kai', 'Add the budget command', [L + 'src/commands.rs', L + 'src/output.rs']],
  ['branch', 'fix/moon-epoch'],
  ['mia', 'Start the Moon table at J2000', [E + 'src/moon.rs']],
  ['merge', 'fix/moon-epoch', 'mia'],
  ['noor', 'Check formatting in CI', ['.github/workflows/ci.yml']],
  ['sam', 'Add an example from low orbit to geostationary', [T + 'examples/leo_to_geo.rs']],
  ['release', 'ada', '0.3.0'],
  ['branch', 'feature/launch-windows'],
  ['mia', 'Add phase angles for transfers', [T + 'src/window.rs']],
  ['sam', "Find launch windows from the Sun's direction", [T + 'src/window.rs']],
  ['mia', 'Explain launch windows in the guide', ['docs/transfers.md']],
  ['merge', 'feature/launch-windows', 'sam'],
  ['kai', 'Add --json for scripts', [L + 'src/output.rs', L + 'src/args.rs']],
  ['kai', 'Name the --svg file in the usage', [L + 'src/args.rs']],
  ['ada', 'Write a guide for new contributors', ['docs/contributing.md']],
  ['noor', 'Keep SVG drawings out of Git', ['.gitignore']],
  ['ada', 'Document every public item in orbit-core', [C + 'src/orbit.rs', C + 'src/state.rs']],
  ['mia', 'Wrap angles into -pi to pi', [C + 'src/angle.rs']],
  ['ada', 'Show the plan command in the README', ['README.md']],
  ['release', 'ada', '0.4.0']
];
function buildWorkspace() {
  let seed = 20260504;
  const rand = () => { seed = seed + 0x6D2B79F5 | 0; let t = Math.imul(seed ^ seed >>> 15, 1 | seed); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; };
  const total = new Map(), seen = new Map();
  for (const op of WS_PLAN) if (TEAM[op[0]]) for (const p of op[2]) total.set(p, (total.get(p) || 0) + 1);
  const count = WS_PLAN.filter(op => op[0] !== 'branch').length + 1;
  // The i-th of n versions of a file: its first paragraphs, all of them the last time.
  const grown = (p, i, n) => {
    const parts = FILES[p].split('\n\n'), text = parts.slice(0, Math.max(1, Math.ceil(parts.length * i / n))).join('\n\n');
    return text.endsWith('\n') ? text : text + '\n';
  };
  // Dates: spread from May 4 to early November, in each person's working hours and time zone.
  let k = 0, last = 0;
  const stamp = who => {
    const [, off, h0, h1] = TEAM[who];
    let day = Date.UTC(2026, 4, 4) + Math.floor(k++ * 189 / count + rand() * 3) * 86400000;
    const weekday = new Date(day).getUTCDay();
    if (weekday === 6 && rand() < 0.85) day -= 86400000; else if (weekday === 0 && rand() < 0.85) day += 86400000;
    let t = Math.floor(day / 1000 + (h0 + rand() * (h1 - h0)) * 3600 - off * 60);
    if (t <= last) t = last + 300 + Math.floor(rand() * 1800);
    last = t;
    const a = Math.abs(off);
    return t + ' ' + (off < 0 ? '-' : '+') + String(Math.floor(a / 60)).padStart(2, '0') + String(a % 60).padStart(2, '0');
  };
  const data = text => 'data ' + Buffer.byteLength(text) + '\n' + text + '\n';
  let stream = '', mark = 0, on = 'main', version = '0.1.0';
  const trees = { main: new Map() }, tips = {}, forks = {}, released = [], merged = [];
  function commit(ref, who, message, parents, before, after) {
    const id = `${TEAM[who][0]} <${who}@example.invalid> ${stamp(who)}`;
    stream += `commit refs/heads/${ref}\nmark :${++mark}\nauthor ${id}\ncommitter ${id}\n` + data(message) + parents.map((p, i) => (i ? 'merge :' : 'from :') + p + '\n').join('');
    for (const [p, v] of after) if (before.get(p) !== v) stream += `M 100644 inline ${p}\n` + data(v);
    stream += '\n';
    return mark;
  }
  const roots = tree => {
    for (const name of Object.keys(PKG)) {
      const has = m => tree.has(`crates/${name}/src/${m}.rs`);
      if (PKG[name].mods.some(has)) tree.set(`crates/${name}/src/${PKG[name].root || 'lib.rs'}`, crateRoot(name, has));
    }
  };
  for (const op of WS_PLAN) {
    if (op[0] === 'branch') { trees[op[1]] = new Map(trees[on]); forks[op[1]] = trees[on]; tips[op[1]] = tips[on]; on = op[1]; continue; }
    const before = op[0] === 'merge' ? trees.main : trees[on], after = new Map(before);
    if (op[0] === 'merge') {
      const [, branch, who] = op, base = forks[branch];
      for (const [p, v] of trees[branch]) if (base.get(p) !== v) {
        assert(/src\/(lib|main)\.rs$/.test(p) || trees.main.get(p) === base.get(p), 'both sides changed ' + p);
        after.set(p, v);
      }
      roots(after);
      tips.main = commit('main', who, `Merge branch '${branch}'`, [tips.main, tips[branch]], trees.main, after);
      trees.main = after; on = 'main'; merged.push(branch); continue;
    }
    if (op[0] === 'release') {
      const [, who, v] = op; version = v; released.unshift(v);
      after.set('Cargo.toml', WORKSPACE(v)); after.set('CHANGELOG.md', CHANGELOG(released));
      tips[on] = commit(on, who, 'Release ' + v, [tips[on]], before, after); trees[on] = after;
      stream += `reset refs/tags/v${v}\nfrom :${tips[on]}\n\n`; continue;
    }
    const [who, message, paths] = op;
    for (const p of paths) {
      const i = (seen.get(p) || 0) + 1, text = grown(p, i, total.get(p)); seen.set(p, i);
      assert.notEqual(text, before.get(p), `"${message}" leaves ${p} unchanged`); after.set(p, text);
    }
    if (!after.has('Cargo.toml')) after.set('Cargo.toml', WORKSPACE(version));
    roots(after);
    tips[on] = commit(on, who, message, tips[on] ? [tips[on]] : [], before, after); trees[on] = after;
  }
  for (const [p, v] of trees.main) assert(!FILES[p] || v === FILES[p], p + ' did not reach its last version');
  // The marked branch, one commit past main, so that the Map can show main without its name.
  const next = new Map(trees.main); next.set('CHANGELOG.md', CHANGELOG(released, true));
  commit(MARK, 'ada', 'Start the notes for the next release', [tips.main], trees.main, next);
  const gitD = (args, input) => cp.execFileSync('git', ['-C', repoD, ...args], { env, input, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
  gitD(['init', '--quiet', '--initial-branch=main']);
  for (const [k2, v] of [['core.autocrlf', 'false'], ['commit.gpgsign', 'false'], ['core.hooksPath', path.join(root, 'empty-template')],
    ['user.name', TEAM.ada[0]], ['user.email', 'ada@example.invalid']]) gitD(['config', k2, v]);
  gitD(['fast-import', '--quiet'], stream);
  gitD(['update-ref', '--stdin'], merged.map(b => `delete refs/heads/${b}\n`).join(''));
  gitD(['symbolic-ref', 'HEAD', 'refs/heads/' + MARK]); gitD(['reset', '--quiet', '--hard']);
  write(repoD, '.git/rocket-ui-fixture', MARK);
  console.error(`workspace: ${mark} commits`);
}

// What the stand-in agent "wrote" in its worktree of repository A: --json for the plan command.
const PLAN_JSON = `use orbit_planner::{hohmann, Orbit};

fn main() {
    let mut json = false;
    let mut altitudes = Vec::new();
    for arg in std::env::args().skip(1) {
        if arg == "--json" {
            json = true;
        } else {
            altitudes.push(arg.parse::<f64>());
        }
    }
    let [Ok(from), Ok(to)] = altitudes.as_slice() else {
        eprintln!("usage: plan [--json] <from-km> <to-km>");
        std::process::exit(2);
    };
    let transfer = hohmann(Orbit::around_earth(*from), Orbit::around_earth(*to));
    if json {
        println!(
            r#"{{"departure":{:.3},"arrival":{:.3},"total":{:.3},"coast":{:.0}}}"#,
            transfer.departure_burn, transfer.arrival_burn, transfer.total(), transfer.coast
        );
        return;
    }
    println!("departure {:.3} km/s", transfer.departure_burn);
    println!("arrival   {:.3} km/s", transfer.arrival_burn);
    println!("total     {:.3} km/s", transfer.total());
    println!("coast     {:.0} s", transfer.coast);
}
`;
const PLAN_JSON_TEST = `use std::process::Command;

#[test]
fn json_output_has_every_number() {
    let out = Command::new(env!("CARGO_BIN_EXE_plan"))
        .args(["--json", "400", "35786"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    for key in ["departure", "arrival", "total", "coast"] {
        assert!(text.contains(&format!("\\"{key}\\":")), "{key} is missing from {text}");
    }
}
`;

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
// The Agents, Map and Insights controls are kept to place those pictures' crops.
const KEPT = /^(diff-line-\d+$|diff-hunk-|diff-apply-selection$|diff-split$|int-play-run$|int-play-ui-|palette-input$|palette-row-|commit-[0-9a-f]{7,}$|conflict-ai$|conflict-ai-apply$|agents-|map-|stats-)/;
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

// Agents, on repository A. Under MCP a session runs only rocket-mcp's own stand-in program, on a
// rocket-ui-test/ branch, and the session's header names that branch. So the site shows the session
// list and the Changes pane under the header, as two crops of one shot, and never the header itself.
const AGENT_TASKS = [ // folder, task name, preset, what the stand-in reports
  ['json', 'Add JSON output to plan', 'codex', ['working', 'complete']],
  ['moon', 'Plan transfers around the Moon', 'codex', ['working']],
  ['coast', 'Check coast times against a table', 'custom', []]
];
let agentIds = null;
const session = id => state.agents?.sessions?.find(s => s.id === id);
async function scrollOn(target, direction, pixels) {
  await client.tool('rocket_scroll', { target, direction, pixels, expected_tab: state.active_tab }); await until(s => !s.input_pending, 'scroll'); await stable();
}
async function agentReveal(id) {
  await stable();
  const visible = () => { const c = control(id); return Boolean(c && c.rect[3] - c.rect[1] > 18); };
  const target = state.agents?.form ? 'agents-scroll' : id.startsWith('agents-session-') ? 'agents-list-scroll' : id.startsWith('agents-change-') ? 'agents-changes-scroll' : null;
  if (visible() || !target) return;
  await scrollOn(target, 'up', 2000);
  for (let i = 0; i < 18 && !visible(); i++) await scrollOn(target, 'down', 180);
}
const agentsIdle = label => until(s => !s.busy && !s.input_pending && !s.agents?.busy && !s.agents?.form?.reading, label, 60000);
async function agentClick(id) { await agentReveal(id); await click(id); await agentsIdle(id); }
async function agentType(id, text) { await agentReveal(id); await type(id, text); }
async function startAgents() {
  fs.mkdirSync(path.join(root, 'agents'), { recursive: true });
  const ids = {};
  for (const [key, title, preset, reports] of AGENT_TASKS) {
    const folder = path.join(root, 'agents', key), branch = 'rocket-ui-test/agent-' + key;
    fs.writeFileSync(folder + '.rocket-worktree-fixture', branch + '\n' + repoA);
    await agentClick('agents-new'); await until(s => s.agents?.form, 'New session');
    await agentClick('agents-provider-' + preset); await stable();
    if (!control('agents-program') && control('agents-advanced')) await agentClick('agents-advanced');
    await agentType('agents-name', title); await agentType('agents-base', 'main'); await agentType('agents-branch', branch);
    await agentType('agents-destination', folder); await agentType('agents-program', exe);
    await agentType('agents-arguments', preset === 'codex' ? '-c\nrocket_agent_fixture=lifecycle' : '--agent-session-fixture\nhold');
    await agentClick('agents-review'); assert(state.agents.form?.reviewed, 'agent review: ' + state.agents.form?.error);
    await agentClick('agents-confirm'); await until(s => !s.agents.form, 'session created');
    const id = ids[key] = state.agents.selected;
    await until(() => session(id)?.phase === 'Running' && (preset === 'codex' ? session(id).activity_state === 'Ready for input' : session(id).tail?.includes('SYNTHETIC_AGENT_READY')), key + ' running', 60000);
    for (const report of reports) {
      if (state.agents.pane !== 'Terminal') await agentClick('agents-tab-terminal');
      await agentType('terminal-command', report); await client.tool('rocket_terminal_enter', { expected_tab: state.active_tab });
      const want = { working: 'Working', complete: 'Turn finished' }[report];
      await until(() => session(id)?.activity_state === want, key + ' ' + want);
    }
  }
  // The stand-in changes no files, so what an agent would have written goes into its worktree here.
  const json = path.join(root, 'agents', 'json');
  write(json, 'src/bin/plan.rs', PLAN_JSON); write(json, 'tests/plan.rs', PLAN_JSON_TEST);
  write(json, 'README.md', fs.readFileSync(path.join(json, 'README.md'), 'utf8').replace('Add `--json` for scripts.', 'Add `--json` for one line of JSON with the same four numbers.'));
  return ids;
}
async function agentShot(name) {
  await click('view-agents'); await agentsIdle('Agents');
  if (!agentIds) agentIds = await startAgents();
  const id = agentIds.json;
  if (state.agents.selected !== id) await agentClick('agents-session-' + id);
  if (state.agents.pane !== 'Changes') await agentClick('agents-tab-changes');
  await until(s => s.agents.changes.owner === id && s.agents.changes.loaded && !s.agents.changes.busy && s.agents.changes.files?.length === 3, 'agent changes', 60000);
  const file = state.agents.changes.files.find(f => f.path === 'src/bin/plan.rs');
  if (state.agents.changes.selected !== file.index) await agentClick('agents-change-' + file.index);
  await until(s => s.agents.changes.patch?.includes('--json') && !s.agents.changes.busy, 'plan.rs diff');
  assert.equal(session(id).activity_state, 'Turn finished'); await capture(name);
}
async function stopAgents() {
  for (const id of Object.values(agentIds || {})) {
    if (!session(id)?.active) continue;
    await agentClick('agents-session-' + id); await click('agents-stop', false);
    await until(() => session(id)?.phase === 'Stopped' && !session(id).active, 'agent stopped');
  }
  // Stop reads the worktree's changes again; opening another repository meanwhile is refused.
  await agentsIdle('Agents after Stop');
}

// The Map and Insights, on repository D. Code and People show HEAD, which names no branch. History
// shows main, where the marked branch, one commit ahead, has no label.
const map = () => tab()?.repository_map;
const mapNode = (test, what) => { const n = map().data.nodes.find(test); assert(n, 'no ' + what + ' on the map'); return n; };
const mapRead = label => until(() => map()?.data && !map().running && !state.busy && !state.input_pending, label, 90000);
async function mapShots(suffix) {
  if (tab().view !== 'Map') await click('view-map');
  await click('map-view-code'); await mapRead('Code');
  if (map().kind === 'Modules' || !map().alternate) { await click('map-mode-packages'); await mapRead('Packages'); }
  await click('map-node-' + mapNode(n => n.kind === 'Crate' && n.label === 'orbit-transfer', 'orbit-transfer').index); await capture('map-code' + suffix);
  await click('map-mode-modules'); await until(() => map().kind === 'Modules' && !map().running && map().data?.kind === 'Modules', 'Modules', 90000);
  if (map().data.nodes[map().data.modules.package]?.label !== 'orbit-core') {
    await click('map-module-package');
    await click('map-module-package-' + map().data.nodes.filter(n => n.kind === 'Crate').findIndex(n => n.label === 'orbit-core'));
    await until(() => !map().running && map().data.nodes[map().data.modules.package]?.label === 'orbit-core', 'orbit-core modules', 90000);
  }
  for (const name of ['state', 'orbit']) {
    await click('map-module-' + mapNode(n => n.kind === 'Module' && n.path === C + 'src/' + name + '.rs', name + '.rs').index); await capture(`map-modules-${name}${suffix}`);
  }
  await click('map-view-history'); await mapRead('History');
  if (map().revision !== 'main') { await click('map-revision-pick'); await type('map-revision', 'main'); await click('map-load'); await mapRead('History of main'); }
  assert(!map().data.nodes.some(n => n.refs.some(r => r.includes('rocket-ui-test'))), 'the marked branch shows on the map');
  await capture('map-history' + suffix);
  await click('map-view-people'); await mapRead('People');
  await click('map-node-' + mapNode(n => n.kind === 'Person' && n.label === 'Sam Example', 'Sam').index); await capture('map-people' + suffix);
}
const stats = () => state.dialog?.statistics;
async function insightsShots(suffix) {
  await click('menu-view'); await click('view-goto'); await click('goto-statistics');
  await until(() => stats()?.activity && !stats().activity_running, 'Insights', 90000);
  if (stats().range !== 'all') { await click('stats-range-all'); await until(() => stats().activity && !stats().activity_running, 'all time', 90000); }
  await capture('insights' + suffix);
  await click('stats-section-treemap'); await until(() => stats().treemap && !stats().treemap_running, 'Files', 90000); await capture('insights-files' + suffix);
  await click('dialog-cancel');
}

async function run() {
  build(); buildWorkspace();
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
  // A tab with a message draft or changed files asks before it closes; they stay in its folder.
  for (const other of state.tabs.filter(t => t.id !== state.active_tab)) {
    await click('close-tab-' + other.id); if (state.dialog?.kind === 'close-tab') await click('dialog-submit');
  }
  assert.equal(state.tabs.length, 1, 'the conflict shots need the other tab closed');
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

  // Agents and the Map come last, so they cannot unsettle the steps above.
  await client.tool('rocket_open', { path: repoA }); await settled(); if (control('console-close')) await click('console-close');
  await attempt('agents', async () => { await look('dark'); await agentShot('agents'); });
  await attempt('agents-light', async () => { await look('light'); await agentShot('agents-light'); });
  try { await stopAgents(); } catch (e) { report.failed.push({ name: 'agents-stop', error: String(e.message || e).slice(0, 400) }); }
  await client.tool('rocket_open', { path: repoD }); await settled(); if (control('console-close')) await click('console-close');
  await attempt('map', async () => { await look('dark'); await mapShots(''); await insightsShots(''); });
  await attempt('map-light', async () => { await look('light'); await mapShots('-light'); await insightsShots('-light'); });
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
