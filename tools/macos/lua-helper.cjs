'use strict';
// Build-time packaging for the fixed Lua/Pulley helper. Never downloads code.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const assert = require('node:assert/strict'), crypto = require('node:crypto');
const ID = 'io.github.jasxtd.rocket.lua';
const FILES = ['Contents/Info.plist', 'Contents/MacOS/rocket-lua-host', 'Contents/_CodeSignature/CodeResources'];
const EXE = FILES[1], PREFIX = 'Rocket.app/Contents/Helpers/RocketLua.app/';
const sha = data => crypto.createHash('sha256').update(data).digest('hex');
function plist() { return `<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${ID}</string><key>CFBundleExecutable</key><string>rocket-lua-host</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>1</string><key>LSUIElement</key><true/></dict></plist>\n`; }
function inspect(app, read, walk, macho) {
  assert.deepEqual(walk(app), FILES, 'Lua helper inventory mismatch');
  assert.equal(read(path.join(app, FILES[0]), 32768).toString(), plist(), 'Lua helper metadata mismatch');
  const bytes = read(path.join(app, EXE), 64 * 1024 * 1024); macho(bytes);
  assert(read(path.join(app, FILES[2]), 1024 * 1024).length > 0, 'Missing Lua resource seal');
  return sha(bytes);
}
function native(app) {
  assert.equal(process.platform, 'darwin');
  const run = (args) => {
    const r = cp.spawnSync('/usr/bin/codesign', args, { encoding: 'utf8', timeout: 30000, maxBuffer: 1024 * 1024 });
    assert.ifError(r.error); assert.equal(r.status, 0, r.stderr); return r;
  };
  run(['--verify', '--strict', '--deep', app]);
  const ent = run(['--display', '--entitlements', ':-', app]).stdout;
  // Ask the OS parser, not an XML substring test, for the exact entitlement set.
  const parsed = cp.spawnSync('/usr/bin/plutil', ['-convert', 'json', '-o', '-', '--', '-'], { input: ent, encoding: 'utf8', timeout: 10000 });
  assert.ifError(parsed.error); assert.equal(parsed.status, 0, parsed.stderr);
  assert.deepEqual(JSON.parse(parsed.stdout), { 'com.apple.security.app-sandbox': true });
  const identity = run(['--display', '--verbose=4', app]).stderr;
  assert(identity.includes('Identifier=' + ID + '\n') && /flags=.*\bruntime\b/.test(identity), 'Lua helper must have its fixed identity and hardened runtime');
}
function prepare(binary, app) {
  assert.equal(process.platform, 'darwin');
  const b = require('./bundle.cjs'), bytes = b.read(binary, 64 * 1024 * 1024); b.macho(bytes,process.arch);
  assert(!fs.existsSync(app));
  fs.mkdirSync(path.join(app, 'Contents/MacOS'), { recursive: true, mode: 0o755 });
  fs.writeFileSync(path.join(app, EXE), bytes, { flag: 'wx', mode: 0o755 });
  fs.writeFileSync(path.join(app, FILES[0]), plist(), { flag: 'wx', mode: 0o644 });
  const entitlements = path.join(path.dirname(app), path.basename(app) + '.entitlements.plist');
  fs.writeFileSync(entitlements, '<?xml version="1.0"?><plist version="1.0"><dict><key>com.apple.security.app-sandbox</key><true/></dict></plist>', { flag: 'wx', mode: 0o600 });
  const r = cp.spawnSync('/usr/bin/codesign', ['--force', '--sign', '-', '--timestamp=none', '--options', 'runtime', '--identifier', ID, '--entitlements', entitlements, app], { encoding: 'utf8', timeout: 30000 });
  assert.ifError(r.error); assert.equal(r.status, 0, r.stderr);
  // The signed bytes are final. Build hosts with this digest, never sign this
  // helper again during outer bundle finalization (including Developer ID).
  native(app);
  return inspect(app, b.read, b.walk, b.macho);
}
module.exports = { ID, FILES, EXE, PREFIX, plist, inspect, native, prepare };
if (require.main === module) {
  try {
    const [command, source, app] = process.argv.slice(2);
    assert.equal(command, 'prepare'); assert(source && app && path.isAbsolute(source) && path.isAbsolute(app));
    console.log(prepare(source, app));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
