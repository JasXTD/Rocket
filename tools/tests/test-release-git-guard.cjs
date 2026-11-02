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
  for (const [version, expected] of [['2.56.0.windows.3', 'ssh'], ['2.56.0.windows.4', 'https:ssh'], ['2.56.0', 'ssh'], ['2.55.0.windows.1', 'ssh']]) {
    for (const protocol of ['https', 'ssh']) {
      const marker = path.join(root, `policy-${report.cases.length}.txt`), destination = path.join(root, `destination-${report.cases.length}`);
      const env = { ...process.env };
      for (const name of Object.keys(env)) if (/^(GIT_|SSH_|GCM_)|_PROXY$/i.test(name)) delete env[name];
      Object.assign(env, { ROCKET_SECURITY_TEST_GIT_VERSION: version, ROCKET_SECURITY_TEST_MARKER: marker });
      const result = cp.spawnSync(cli, ['--json', '--no-log', '--timeout-secs', '5', '--git', git, 'clone', '--url', `${protocol}://audit.invalid/repo.git`, destination], { env, windowsHide: true, encoding: 'utf8', timeout: 15000 });
      if (result.error) throw result.error;
      assert.notEqual(result.status, 0, 'fixture deliberately refuses to clone');
      assert(fs.existsSync(marker), 'owned Git fixture was not reached: ' + result.stdout + result.stderr);
      const policy = fs.readFileSync(marker, 'utf8');
      report.cases.push({ version, protocol, policy, expected });
      assert.equal(policy, expected, `Wrong protocol policy for Git ${version}`);
      console.log(`PASS Git ${version}, ${protocol} request: ${policy}`);
    }
  }
  report.passed = true;
} catch (error) { report.error = String(error.stack || error); process.exitCode = 1; }
finally { fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2)); }
