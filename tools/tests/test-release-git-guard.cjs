// Exercise the packaged CLI with owned Rust code; no real Git server is contacted.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto'), cp = require('node:child_process'), assert = require('node:assert/strict');
const version = fs.readFileSync(path.join(__dirname, '../../Cargo.toml'), 'utf8').match(/\[workspace\.package\][\s\S]*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/)[1];
const cli = path.resolve(process.argv[2] || path.join(__dirname, '../releases', version, 'rocket-cli.exe'));
const root = path.resolve('work/security-git-guard/' + crypto.randomUUID());
fs.mkdirSync(root, { recursive: true });
const git = path.join(root, 'git.exe');
const compile = cp.spawnSync('rustc', ['--edition=2024', '-C', 'opt-level=1', '-C', 'debuginfo=0', 'tools/helpers/security-git/main.rs', '-o', git], { windowsHide: true, encoding: 'utf8', timeout: 30000 });
if (compile.error) throw compile.error;
assert.equal(compile.status, 0, compile.stdout + compile.stderr);
const report = { passed: false, cli, cli_sha256: crypto.createHash('sha256').update(fs.readFileSync(cli)).digest('hex'), root, cases: [] };
try {
  // Local commands also need the merge and patch fixes, even when no transport runs.
  const local = cp.spawnSync(cli, ['--json', '--no-log', '--git', git, '--repo', root, 'info'], {
    env: { ...process.env, ROCKET_SECURITY_TEST_GIT_VERSION: '2.56.0.windows.4' },
    windowsHide: true, encoding: 'utf8', timeout: 15000,
  });
  if (local.error) throw local.error;
  assert.notEqual(local.status, 0);
  assert.equal(JSON.parse(local.stdout).error.code, 'unsupported');
  console.log('PASS Git 2.56.0.windows.4 refused for local repository discovery');
  for (const [version, allowed] of [['2.56.0.windows.4', false], ['2.56.1', false], ['2.56.1.windows.0', false], ['2.56.1.windows.1', true], ['2.57.0.windows.1', true]]) {
    for (const protocol of ['https', 'ssh']) {
      const marker = path.join(root, `policy-${report.cases.length}.txt`), destination = path.join(root, `destination-${report.cases.length}`);
      const env = { ...process.env };
      for (const name of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(name)) delete env[name];
      Object.assign(env, { ROCKET_SECURITY_TEST_GIT_VERSION: version, ROCKET_SECURITY_TEST_MARKER: marker });
      const result = cp.spawnSync(cli, ['--json', '--no-log', '--timeout-secs', '5', '--git', git, 'clone', '--url', `${protocol}://audit.invalid/repo.git`, destination], { env, windowsHide: true, encoding: 'utf8', timeout: 15000 });
      if (result.error) throw result.error;
      assert.notEqual(result.status, 0, 'fixture deliberately refuses to clone');
      const policy = fs.existsSync(marker) ? fs.readFileSync(marker, 'utf8') : null;
      assert.equal(policy, allowed ? 'https:ssh' : null, `Wrong clone decision for Git ${version}`);
      if (!allowed) assert.match(result.stdout + result.stderr, /Git for Windows 2\.56\.1\.windows\.1 or later required/);
      report.cases.push({ version, protocol, allowed, policy });
      console.log(`PASS Git ${version}, ${protocol}: ${allowed ? policy : 'refused during discovery'}`);
    }
  }
  report.passed = true;
} catch (error) { report.error = String(error.stack || error); process.exitCode = 1; }
finally { fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2)); }
