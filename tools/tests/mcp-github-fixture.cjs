// Setup/checks only. The operation under test is sent through Codex's connected MCP.
// No credentials are requested, returned, or written by this helper.
const fs = require('node:fs');
const path = require('node:path');
const { createHash, randomUUID } = require('node:crypto');
const { spawnSync } = require('node:child_process');
fs.mkdirSync(path.resolve('work/mcp-fixtures'), {recursive:true});
const parent = fs.realpathSync(path.resolve('work/mcp-fixtures'));
const windows = process.platform === 'win32';
// Linux runs local mode only, with the system Git; the GitHub modes need Windows.
const gitExe = windows ? 'C:\\Program Files\\Git\\cmd\\git.exe' : '/usr/bin/git';
const sshDir = windows ? path.join(process.env.WINDIR, 'System32', 'OpenSSH') : '/usr/bin';
const askpass = windows ? 'deny-askpass.cmd' : 'deny-askpass';
const repository = 'JasXTD/rocket-auth-test';
const hostKey = 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl';
const hostFingerprint = 'SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU';
const hostKeySource = 'https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints';
const ensure = (ok, message) => { if (!ok) throw Error(message); };
const hash = file => createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const shellQuote = value => "'" + value.replaceAll('\\', '/').replaceAll("'", "'\"'\"'") + "'";
const sshQuote = value => {
  ensure(!/[\r\n"]|\$\{/.test(value), 'Unsupported SSH fixture path');
  return '"' + value.replaceAll('\\', '/').replaceAll('%', '%%') + '"';
};

class Fixture {
  constructor(root, { gitExecutable = gitExe } = {}) {
    // A caller may test a packaged runtime without changing system Git or PATH.
    // This is supplied by the reviewed runner, never loaded from fixture JSON.
    ensure(path.isAbsolute(gitExecutable) && fs.statSync(gitExecutable).isFile(), 'Expected an explicit installed Git executable');
    this.gitExecutable = gitExecutable;
    this.root = fs.realpathSync(root);
    ensure(path.dirname(this.root) === parent && /^network-(https|ssh|local)-[a-f0-9]{32}$/.test(path.basename(this.root)), 'Fixture root outside scope');
    this.file = path.join(this.root, 'fixture.json');
    this.r = JSON.parse(fs.readFileSync(this.file, 'utf8'));
    this.mode = this.r.mode;
    ensure(['https', 'ssh', 'local'].includes(this.mode) && this.r.root === this.root, 'Fixture identity mismatch');
    this.branch = 'rocket-ui-test/' + path.basename(this.root);
    this.ref = 'refs/heads/' + this.branch;
    this.tracking = 'refs/remotes/origin/' + this.branch;
    this.client = path.join(this.root, 'ROCKET_PRIVATE_client');
    this.publisher = path.join(this.root, 'ROCKET_PRIVATE_publisher');
    this.hooks = path.join(this.root, 'empty-hooks');
    this.remote = this.mode === 'local' ? path.join(this.root, 'upstream.git') : this.mode === 'https' ? `https://github.com/${repository}.git` : `git@github.com:${repository}.git`;
    this.env = { ...process.env };
    for (const key of Object.keys(this.env)) if (/^(GIT_|GCM_|ROCKET_|SSH_|CURL_|SSL_CERT_)|_PROXY$|^NETRC$/i.test(key)) delete this.env[key];
    Object.assign(this.env, { HOME: path.join(this.root, 'empty-home'), GIT_CONFIG_GLOBAL: path.join(this.root, 'empty.config'), GIT_CONFIG_SYSTEM: path.join(this.root, 'empty.config'),
      GIT_CONFIG_NOSYSTEM: '1', GIT_TEMPLATE_DIR: this.hooks, GIT_TERMINAL_PROMPT: '0', GIT_OPTIONAL_LOCKS: '0', GIT_NO_LAZY_FETCH: '1', GIT_NO_REPLACE_OBJECTS: '1',
      GIT_ASKPASS: path.join(this.root, askpass), SSH_ASKPASS: path.join(this.root, askpass), SSH_ASKPASS_REQUIRE: 'never',
      GIT_ALLOW_PROTOCOL: this.mode === 'local' ? 'file' : this.mode, GCM_INTERACTIVE: '0', GCM_TRACE: '0', GCM_TRACE_SECRETS: '0', GCM_DEBUG: '0', LC_ALL: 'C' });
  }
  save() { fs.writeFileSync(this.file, JSON.stringify(this.r, null, 2)); }
  tool(exe, args, label) {
    const result = spawnSync(exe, args, { cwd: this.root, env: this.env, input: '', encoding: 'utf8', windowsHide: true, timeout: 45000, maxBuffer: 2 * 1024 * 1024 });
    this.r.steps.push({ phase: this.r.phase, tool: label, exit: result.status, timed_out: result.error?.code === 'ETIMEDOUT' });
    this.save();
    ensure(result.status === 0, `${label} failed during ${this.r.phase}; raw transport output was not persisted`);
    return result.stdout.trimEnd();
  }
  git(args, at = this.client) { return this.tool(this.gitExecutable, ['-C', at, '-c', 'gc.auto=0', '-c', 'maintenance.auto=false', ...args], 'git-' + args[0]); }
  config(at, key, value) { this.git(['config', '--local', '--replace-all', key, value], at); }
  assertScope() {
    for (const at of [this.client, this.publisher]) {
      ensure(fs.lstatSync(at).isDirectory() && !fs.lstatSync(at).isSymbolicLink(), 'Redirected fixture directory');
      ensure(fs.lstatSync(path.join(at, '.git')).isDirectory() && !fs.lstatSync(path.join(at, '.git')).isSymbolicLink(), 'Redirected Git directory');
      ensure(this.git(['config', '--get', 'remote.origin.url'], at) === this.remote, 'Origin changed; preserved');
      ensure(this.git(['config', '--get', 'remote.origin.fetch'], at) === `+${this.ref}:${this.tracking}`, 'Fetch mapping changed; preserved');
    }
    ensure(fs.readFileSync(path.join(this.client, '.git', 'rocket-ui-fixture'), 'utf8') === this.branch, 'Ownership marker changed');
  }
  auth(enabled) {
    if (this.mode === 'local') return;
    // Only the client is denied. The publisher can still verify and clean up.
    this.configureAuth(this.client, enabled);
    this.r.client_auth = enabled;
  }
  configureAuth(at, enabled) {
    if (this.mode === 'local') return;
    if (this.mode === 'https') {
      this.config(at, 'credential.helper', '');
      if (enabled) {
        const helper = '!f() { if test "$1" = get; then GCM_INTERACTIVE=0 GCM_TRACE=0 GCM_TRACE_SECRETS=0 GCM_DEBUG=0 ' + shellQuote(this.gitExecutable) + ' credential-manager get; else cat >/dev/null; fi; }; f';
        this.git(['config', '--local', '--add', 'credential.helper', helper], at);
      }
      return;
    }
    const name = at === this.client ? 'client' : 'publisher';
    const cfg = path.join(this.root, name + '-ssh.config');
    const known = path.join(this.root, 'known_hosts');
    const lines = ['Host github.com', '  HostName github.com', '  User git', '  Port 22', '  BatchMode yes', '  ConnectTimeout 10', '  ConnectionAttempts 1',
      '  StrictHostKeyChecking yes', '  HostKeyAlgorithms ssh-ed25519', '  UserKnownHostsFile ' + sshQuote(known), '  GlobalKnownHostsFile none',
      '  UpdateHostKeys no', '  VerifyHostKeyDNS no', '  PreferredAuthentications publickey', '  PasswordAuthentication no', '  KbdInteractiveAuthentication no',
      '  IdentitiesOnly yes', '  AddKeysToAgent no', '  ForwardAgent no', '  ControlMaster no', '  ControlPath none'];
    if (enabled) lines.push('  IdentityFile ' + sshQuote(path.join(this.root, 'identity.pub')));
    else lines.push('  IdentityAgent none', '  IdentityFile none');
    fs.writeFileSync(cfg, lines.join('\n') + '\n');
    this.config(at, 'core.sshCommand', shellQuote(path.join(sshDir, 'ssh.exe')) + ' -F ' + shellQuote(cfg));
  }
  sshPreflight() {
    const original = path.join(process.env.USERPROFILE, '.ssh', 'id_ed25519_github.pub');
    const publicText = fs.readFileSync(original, 'utf8').trim();
    ensure(/^ssh-ed25519 [A-Za-z0-9+/]+={0,3}(?: [^\r\n]*)?$/.test(publicText), 'Expected one Ed25519 public key');
    this.r.public_key_path = original; this.r.public_key_sha256 = hash(original);
    fs.writeFileSync(path.join(this.root, 'identity.pub'), publicText + '\n');
    fs.writeFileSync(path.join(this.root, 'known_hosts'), hostKey + '\n');
    const key = this.tool(path.join(sshDir, 'ssh-keygen.exe'), ['-lf', path.join(this.root, 'identity.pub')], 'public-key-fingerprint').match(/SHA256:[A-Za-z0-9+/]+/)?.[0];
    const agent = this.tool(path.join(sshDir, 'ssh-add.exe'), ['-l'], 'ssh-agent-list');
    ensure(key && agent.includes(key), 'Load id_ed25519_github into the Windows SSH agent locally; no private key was read');
    ensure(this.tool(path.join(sshDir, 'ssh-keygen.exe'), ['-lf', path.join(this.root, 'known_hosts')], 'host-key-fingerprint').includes(hostFingerprint), 'Pinned host key mismatch');
    this.r.selected_agent_key_available = true; this.r.host_key_source = hostKeySource;
  }
  remoteState() {
    const text = this.git(['ls-remote', '--refs', '--', 'origin'], this.publisher);
    const refs = text ? text.split(/\r?\n/).sort() : [];
    ensure(refs.every(r => /^[a-f0-9]{40,64}\trefs\/\S+$/.test(r)), 'Invalid remote ref response');
    return { refs, head: this.git(['ls-remote', '--symref', '--', 'origin', 'HEAD'], this.publisher) };
  }
  tip(state) { return state.refs.find(r => r.endsWith('\t' + this.ref))?.split('\t')[0] || ''; }
  assertRemote(expected) {
    const state = this.remoteState();
    ensure(this.tip(state) === expected, 'Unexpected test branch tip; no write retried');
    ensure(JSON.stringify(state.refs.filter(r => !r.endsWith('\t' + this.ref))) === JSON.stringify(this.r.baseline.refs) && state.head === this.r.baseline.head, 'Other refs or default HEAD changed; preserved');
    return state;
  }
  snapshot() {
    return { head: this.git(['rev-parse', 'HEAD']), branches: this.git(['for-each-ref', '--format=%(refname) %(objectname)', 'refs/heads/', 'refs/tags/']),
      index: hash(path.join(this.client, '.git', 'index')), entries: this.git(['ls-files', '--stage', '-z']),
      files: fs.readdirSync(this.client, { withFileTypes: true }).filter(f => f.isFile()).map(f => [f.name, hash(path.join(this.client, f.name))]).sort() };
  }
  unchanged() { ensure(JSON.stringify(this.snapshot()) === JSON.stringify(this.r.before), 'Local HEAD, index, branches or file bytes changed unexpectedly'); }
  commit(at, name, contents) {
    fs.writeFileSync(path.join(at, name), contents + '\n'); this.git(['add', '--', name], at); this.git(['commit', '-m', 'Rocket synthetic MCP transport fixture'], at);
    const oid = this.git(['rev-parse', 'HEAD'], at); this.r.owned.push(oid); this.save(); return oid;
  }
  leasedWrite(oid, expected) {
    ensure(this.r.baseline && (!oid || this.r.owned.includes(oid)) && (!expected || this.r.owned.includes(expected)), 'Write outside owned commits');
    this.r.remote_write_attempted = true; this.save();
    this.git(['-c', 'remote.origin.mirror=false', '-c', 'push.followTags=false', '-c', 'push.autoSetupRemote=false', '-c', 'push.negotiate=false',
      'push', '--porcelain', '--no-follow-tags', '--no-prune', '--no-recurse-submodules', `--force-with-lease=${this.ref}:${expected}`, '--', 'origin', `${oid}:${this.ref}`], this.publisher);
  }
  syncPublisher(first = false) {
    this.git(['fetch', '--no-tags', '--no-prune', '--no-recurse-submodules', '--', 'origin', `${this.ref}:${this.tracking}`], this.publisher);
    this.git(first ? ['switch', '--create', this.branch, '--no-track', this.tracking] : ['merge', '--ff-only', '--no-edit', this.tracking], this.publisher);
  }
  run(action) {
    this.r.phase = action; this.save();
    if (action !== 'setup') this.assertScope();
    if (action === 'setup') {
      for (const dir of [this.client, this.publisher, this.hooks, path.join(this.root, 'empty-home')]) fs.mkdirSync(dir);
      fs.writeFileSync(path.join(this.root, 'empty.config'), ''); fs.writeFileSync(path.join(this.root, askpass), windows ? '@exit /b 1\r\n' : '#!/bin/sh\nexit 1\n', { mode: 0o755 });
      for (const at of [this.client, this.publisher]) {
        this.git(['init', '--initial-branch=' + this.branch, '--object-format=sha1'], at);
        for (const [key, value] of [['user.name', 'Rocket MCP Test'], ['user.email', 'rocket@example.invalid'], ['core.hooksPath', this.hooks], ['core.attributesFile', path.join(this.root, 'empty.config')],
          ['core.autocrlf', 'false'], ['core.fsmonitor', 'false'], ['core.askPass', path.join(this.root, askpass)], ['commit.gpgsign', 'false'], ['push.gpgsign', 'false'],
          ['credential.interactive', 'false'], ['credential.helper', ''], ['credential.useHttpPath', 'false'], ['http.sslBackend', 'schannel'], ['http.sslVerify', 'true'], ['http.followRedirects', 'false'], ['http.extraHeader', ''],
          ['protocol.allow', 'never'], ['protocol.' + (this.mode === 'local' ? 'file' : this.mode) + '.allow', 'always']]) this.config(at, key, value);
        this.git(['remote', 'add', 'origin', this.remote], at); this.config(at, 'remote.origin.fetch', `+${this.ref}:${this.tracking}`);
      }
      fs.writeFileSync(path.join(this.client, '.git', 'rocket-ui-fixture'), this.branch);
      this.r.base = this.commit(this.client, 'ROCKET_PRIVATE_file', 'Synthetic MCP base');
      if (this.mode === 'local') { this.git(['init', '--bare', '--initial-branch=main', this.remote]); this.git(['push', 'origin', 'HEAD:refs/heads/main']); }
      if (this.mode === 'ssh') this.sshPreflight();
      for (const at of [this.client, this.publisher]) this.configureAuth(at, true);
      this.r.baseline = this.remoteState(); ensure(this.r.baseline.refs.length && this.r.baseline.head, 'Use an initialized test repository');
      ensure(!this.tip(this.r.baseline), 'Generated branch exists; preserved');
      this.r.expected_tip = ''; this.r.before = this.snapshot();
    } else if (action === 'before-push') { this.r.before = this.snapshot(); this.r.remote_write_attempted = true;
    } else if (action === 'verify-new-push') {
      this.unchanged(); this.assertRemote(this.r.base); ensure(this.git(['rev-parse', '--symbolic-full-name', '@{upstream}']) === this.tracking, 'Upstream not configured');
      this.r.expected_tip = this.r.base; fs.writeFileSync(path.join(this.client, 'ROCKET_PRIVATE_untracked'), 'Preserve this synthetic sentinel\n'); this.r.before = this.snapshot();
    } else if (action === 'auth-off' || action === 'auth-on') { this.auth(action === 'auth-on');
    } else if (action === 'checkpoint') { this.assertRemote(this.r.expected_tip); this.r.before = this.snapshot();
    } else if (action === 'verify-unchanged') { this.unchanged(); this.assertRemote(this.r.expected_tip);
    } else if (action === 'publish-advance') {
      this.syncPublisher(true); this.r.advance = this.commit(this.publisher, 'ROCKET_PRIVATE_file', 'Synthetic MCP remote advance');
      this.leasedWrite(this.r.advance, this.r.expected_tip); this.r.expected_tip = this.r.advance; this.assertRemote(this.r.expected_tip);
    } else if (action === 'verify-fetch' || action === 'verify-diverged-fetch') {
      this.unchanged(); this.assertRemote(this.r.expected_tip); ensure(this.git(['rev-parse', this.tracking]) === this.r.expected_tip, 'Tracking ref not refreshed');
      ensure(this.git(['rev-list', '--left-right', '--count', 'HEAD...' + this.tracking]).replace(/\s+/g, ' ') === (action === 'verify-fetch' ? '0 1' : '1 1'), 'Unexpected ahead/behind counts');
    } else if (action === 'verify-pull') {
      ensure(this.git(['rev-parse', 'HEAD']) === this.r.advance && this.git(['rev-parse', this.tracking]) === this.r.advance, 'Pull did not fast-forward');
      ensure(fs.readFileSync(path.join(this.client, 'ROCKET_PRIVATE_file'), 'utf8') === 'Synthetic MCP remote advance\n', 'Pulled file bytes mismatch');
      ensure(fs.readFileSync(path.join(this.client, 'ROCKET_PRIVATE_untracked'), 'utf8') === 'Preserve this synthetic sentinel\n', 'Untracked sentinel changed');
      this.assertRemote(this.r.expected_tip); this.r.before = this.snapshot();
    } else if (action === 'local-advance') {
      this.r.local_advance = this.commit(this.client, 'ROCKET_PRIVATE_client_only', 'Synthetic MCP client advance'); this.r.before = this.snapshot();
    } else if (action === 'verify-local-push') {
      this.unchanged(); this.assertRemote(this.r.local_advance); this.r.expected_tip = this.r.local_advance;
    } else if (action === 'diverge') {
      this.syncPublisher(); this.r.diverged = this.commit(this.publisher, 'ROCKET_PRIVATE_remote_only', 'Synthetic MCP remote divergence');
      this.leasedWrite(this.r.diverged, this.r.expected_tip); this.r.expected_tip = this.r.diverged; this.assertRemote(this.r.expected_tip);
      this.r.local_ahead = this.commit(this.client, 'ROCKET_PRIVATE_client_only', 'Synthetic MCP competing commit'); this.r.before = this.snapshot();
    } else if (action === 'dirty') {
      fs.writeFileSync(path.join(this.client, 'ROCKET_PRIVATE_file'), 'Staged fixture bytes\n'); this.git(['add', '--', 'ROCKET_PRIVATE_file']);
      fs.writeFileSync(path.join(this.client, 'ROCKET_PRIVATE_file'), 'Unstaged fixture bytes\n'); this.r.before = this.snapshot();
    } else if (action === 'cleanup') {
      this.auth(true); this.r.cleanup = 'needs-verification'; this.save(); const tip = this.tip(this.remoteState());
      if (tip) { ensure(this.r.owned.includes(tip), 'Cleanup refused: branch tip is unowned; preserved'); this.leasedWrite('', tip); }
      this.assertRemote(''); this.r.cleanup = 'confirmed-absent'; this.r.original_refs_and_default_head_unchanged = true;
      if (this.mode === 'ssh') ensure(hash(this.r.public_key_path) === this.r.public_key_sha256, 'Original public key changed');
    } else { throw Error('Unknown fixture action'); }
    this.r.checks.push(action); this.save(); return { passed: true, mode: this.mode, root: this.root, client: this.client, branch: this.branch, action, cleanup: this.r.cleanup };
  }
}

function create(mode, options) {
  ensure(['https', 'ssh', 'local'].includes(mode), 'Expected https, ssh or local');
  ensure(windows || mode === 'local', 'The GitHub fixture modes need Windows; Linux runs local mode only');
  const root = path.join(parent, 'network-' + mode + '-' + randomUUID().replaceAll('-', ''));
  fs.mkdirSync(root); fs.writeFileSync(path.join(root, 'fixture.json'), JSON.stringify({ root, mode, repository, phase: 'created', owned: [], steps: [], checks: [], cleanup: 'not-needed' }));
  return new Fixture(root, options);
}
if (require.main === module) {
  let fixture;
  try {
    fixture = process.argv[2] === 'setup' ? create(process.argv[3]) : new Fixture(process.argv[3]);
    console.log(JSON.stringify(fixture.run(process.argv[2])));
  } catch (error) {
    if (fixture) { fixture.r.failure = error.message; fixture.save(); }
    console.error(JSON.stringify({ passed: false, phase: fixture?.r.phase, root: fixture?.root, error: error.message })); process.exitCode = 1;
  }
}
module.exports = { Fixture, create };
