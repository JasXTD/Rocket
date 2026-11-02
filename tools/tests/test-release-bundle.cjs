// Mutates only a disposable copy of an existing local bundle; no network or GUI.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto'), cp = require('node:child_process'), assert = require('node:assert/strict');
const version = fs.readFileSync(path.join(__dirname, '../../Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/)[1];
const source = path.resolve(process.argv[2] || path.join(__dirname, '../releases', version));
const verifier = path.resolve(process.argv[3] || path.join(__dirname, '../Verify-Release.ps1'));
const root = path.resolve('work/security-release-tests/' + crypto.randomUUID());
const bundle = path.join(root, 'bundle');
fs.mkdirSync(root, { recursive: true });
fs.cpSync(source, bundle, { recursive: true, errorOnExist: true, force: false });
fs.mkdirSync(path.join(bundle, 'tools'), {recursive:true});
fs.copyFileSync(verifier, path.join(bundle, 'tools/Verify-Release.ps1'));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const manifest = path.join(bundle, 'SHA256SUMS.txt'), worker = path.join(bundle, 'rocket-lua-host.exe');
function files(directory = '') {
  return fs.readdirSync(path.join(bundle, directory), { withFileTypes: true }).flatMap(entry => {
    const relative = path.join(directory, entry.name);
    assert(!entry.isSymbolicLink(), 'fixture must not contain links');
    return entry.isDirectory() ? files(relative) : relative === 'SHA256SUMS.txt' ? [] : [relative];
  });
}
function refresh() {
  fs.writeFileSync(manifest, files().sort().map(file => hash(path.join(bundle, file)) + '  ' + file.replaceAll('\\', '/')).join('\n') + '\n');
}
function verify(failure, extra = []) {
  const result = cp.spawnSync('pwsh', ['-NoProfile', '-File', path.join(bundle, 'tools/Verify-Release.ps1'), '-Bundle', bundle, ...extra], { windowsHide: true, encoding: 'utf8', timeout: 30000 });
  if (result.error) throw result.error;
  const output = result.stdout + result.stderr;
  if (failure) { assert.notEqual(result.status, 0, 'Unsafe bundle was accepted'); assert.match(output, failure); }
  else { assert.equal(result.status, 0, output); assert.match(output, /Rocket binaries may be unsigned/); }
}
function mutate(file, bytes, operation) {
  const original = fs.readFileSync(file), originalManifest = fs.readFileSync(manifest);
  try { if (bytes === null) fs.unlinkSync(file); else fs.writeFileSync(file, bytes); operation(); }
  finally { fs.writeFileSync(file, original); fs.writeFileSync(manifest, originalManifest); }
}
const report = { passed: false, source, verifier, root, cases: [] };
function pass(name) { report.cases.push(name); console.log('PASS ' + name); }
try {
  refresh(); verify(); pass('Complete unsigned bundle is accepted without a certificate');
  mutate(worker, null, () => { refresh(); verify(/Executable missing from manifest: rocket-lua-host\.exe/); });
  pass('Removing the Lua worker and regenerating the manifest is rejected');
  mutate(worker, Buffer.concat([fs.readFileSync(worker), Buffer.from([0])]), () => { refresh(); verify(/embedded Lua worker hash/); });
  pass('A modified worker is rejected even with a regenerated manifest');
  const host = path.join(bundle, 'rocket-mcp.exe'), digest = Buffer.from(hash(worker));
  const changed = Buffer.from(fs.readFileSync(host)); let offset, replacements = 0;
  while ((offset = changed.indexOf(digest)) >= 0) { changed.fill('f', offset, offset + digest.length); replacements++; }
  assert(replacements > 0, 'host must contain a bound worker digest');
  mutate(host, changed, () => { refresh(); verify(/embedded Lua worker hash/); });
  pass('A host bound to a different worker is rejected');
  mutate(worker, Buffer.concat([fs.readFileSync(worker), Buffer.from([0])]), () => verify(/Hash mismatch/));
  pass('Changed file bytes are rejected by the manifest');
  const extra = path.join(bundle, 'unexpected.exe'), originalManifest = fs.readFileSync(manifest);
  try { fs.copyFileSync(worker, extra); verify(/Unlisted file/); refresh(); verify(/Unexpected executable/); }
  finally { fs.unlinkSync(extra); fs.writeFileSync(manifest, originalManifest); }
  pass('Unlisted and manifest-listed unexpected executables are rejected');
  mutate(manifest, Buffer.from('0'.repeat(64) + '  ../outside\n'), () => verify(/normal relative path/));
  pass('Manifest traversal is rejected');
  verify(/Release blocked/, ['-PublisherThumbprint', '0'.repeat(40)]);
  pass('Optional publisher mode still rejects unsigned binaries');
  verify(undefined, ['-AllowUnsignedLocal']); pass('Existing unsigned-local option remains compatible');
  verify(); report.passed = true;
} catch (error) { report.error = String(error.stack || error); process.exitCode = 1; }
finally { fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2)); }
