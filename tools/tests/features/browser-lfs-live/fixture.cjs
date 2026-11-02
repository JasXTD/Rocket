// Live acceptance setup only. No remote writes, new LFS uploads, or credential stores.
const fs = require('node:fs'), path = require('node:path'), cp = require('node:child_process');
const crypto = require('node:crypto'), assert = require('node:assert/strict');
const { create, Fixture } = require('../../mcp-github-fixture.cjs');
const { payload, oid, name, pointer } = require('../../lfs-github-fixture.cjs');
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const cacheAt = repo => path.join(repo, '.git/lfs/objects', oid.slice(0, 2), oid.slice(2, 4), oid);
function snapshot(f) {
  return { head: f.git(['rev-parse', 'HEAD']), refs: f.git(['for-each-ref', '--format=%(refname) %(objectname)']),
    index: hash(path.join(f.client, '.git/index')), config: f.git(['config', '--local', '--null', '--list']).split('\0')
      .filter(r => !/^lfs\.https:\/\/github\.com\/JasXTD\/rocket-auth-test\.git\/info\/lfs\.access\n(none|basic|private|negotiate)$/i.test(r)).sort(),
    tracked: hash(path.join(f.client, 'ROCKET_PRIVATE_file')), unicode: hash(path.join(f.client, '保留 🚀 e\u0301.txt')) };
}
function personalSnapshot() {
  const base = process.env.USERPROFILE, result = {};
  for (const name of ['.gitconfig', '.ssh/config', '.ssh/known_hosts', '.ssh/id_ed25519_github.pub']) {
    const p = path.join(base, name); result[name] = fs.existsSync(p) ? hash(p) : null;
  }
  // Do not read private keys. Metadata proves this runner didn't replace/edit them.
  const privateKey = path.join(base, '.ssh/id_ed25519_github');
  if (fs.existsSync(privateKey)) { const m = fs.statSync(privateKey); result.privateKeyMetadata = { size: m.size, mtime: m.mtimeMs, birthtime: m.birthtimeMs }; }
  return result;
}
function credentialFingerprint(f) {
  const git = cp.execFileSync('where.exe', ['git.exe'], { encoding: 'utf8', windowsHide: true }).trim().split(/\r?\n/)[0];
  const r = cp.spawnSync(git, ['credential-manager', 'get'], { cwd: f.root, env: f.env, input: 'protocol=https\nhost=github.com\n\n', encoding: 'utf8', windowsHide: true, timeout: 45000, maxBuffer: 128 * 1024 });
  assert.equal(r.status, 0, 'Existing GCM get-only credential check failed; raw output suppressed');
  const token = r.stdout.split(/\r?\n/).find(l => l.startsWith('password='));
  assert(token?.length > 12, 'No existing HTTPS credential; stop before browser test');
  // Only compare in memory. Never return/persist the token or this fingerprint in reports.
  return crypto.createHash('sha256').update(token).digest('hex');
}
function download(f) {
  const p = cacheAt(f.publisher);
  if (fs.existsSync(p)) { assert.equal(hash(p), oid); fs.unlinkSync(p); }
  f.git(['lfs', 'fetch', 'origin', f.r.lfs_pointer_commit], f.publisher);
  assert.equal(hash(p), oid, 'Recovered LFS bytes must match the known synthetic object');
  assert.deepEqual(fs.readFileSync(p), payload);
}
function prepare(mode) {
  assert(['https', 'ssh'].includes(mode));
  const f = create(mode); f.run('setup');
  fs.writeFileSync(path.join(f.client, '保留 🚀 e\u0301.txt'), 'Synthetic untracked Unicode sentinel\n');
  fs.mkdirSync(path.dirname(path.join(f.publisher, name)), { recursive: true });
  fs.writeFileSync(path.join(f.publisher, name), pointer);
  fs.writeFileSync(path.join(f.publisher, '.gitattributes'), '*.dat filter=lfs diff=lfs merge=lfs -text\n');
  f.git(['add', '--', '.gitattributes', name], f.publisher);
  f.git(['commit', '-m', 'Synthetic pointer to existing LFS object; never published'], f.publisher);
  f.r.lfs_pointer_commit = f.git(['rev-parse', 'HEAD'], f.publisher);
  f.r.lfs = { oid, bytes: payload.length, uploads: 0, remote_writes: 0 };
  download(f); // Refuse to seed a prune candidate unless real remote recovery works.
  const p = cacheAt(f.client); fs.mkdirSync(path.dirname(p), { recursive: true }); fs.writeFileSync(p, payload);
  fs.writeFileSync(path.join(f.client, '.git/rocket-lfs-prune-fixture'), `ROCKET-LFS-PRUNE-1\n${f.branch}\n${f.remote}\n${oid}\n${payload.length}\n`);
  fs.writeFileSync(path.join(f.root, 'rocket-private-state'), 'synthetic-content-only-v1\n');
  f.r.prepared_snapshot = snapshot(f); f.r.phase = 'prune-ready'; f.save();
  return f;
}
module.exports = { prepare, Fixture, hash, snapshot, personalSnapshot, credentialFingerprint, download, cacheAt, payload, oid };
if (require.main === module) {
  try { const f = prepare(process.argv[2]); console.log(JSON.stringify({ prepared: true, root: f.root, mode: f.mode, existing_object_bytes: payload.length, remote_writes: 0, uploads: 0 })); }
  catch (e) { console.error(e.message); process.exitCode = 1; }
}
