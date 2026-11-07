'use strict';
// What differs between the Windows and Linux acceptance runs: program names, the shell an
// MCP window runs (Windows PowerShell, or Rocket's bash on Linux) with the exact probes
// rocket-mcp accepts for it (SHELL_PROBES in crates/rocket-mcp/src/ui.rs), and whether a
// process has ended.
const fs = require('node:fs'), cp = require('node:child_process');
const windows = process.platform === 'win32';
const exe = name => windows ? name + '.exe' : name;

// With an image name, a PID that now runs another program counts as ended too: Windows
// hands a freed PID to the next process at once, even to the tasklist run checking it.
function gone(pid, image) {
  if (windows) {
    const only = image ? ['/FI', `IMAGENAME eq ${image}`] : [];
    const r = cp.spawnSync('tasklist.exe', ['/FI', `PID eq ${pid}`, ...only, '/FO', 'CSV', '/NH'], { encoding: 'utf8', windowsHide: true });
    if (r.status !== 0) throw Error('tasklist failed');
    return !r.stdout.includes(`"${pid}"`);
  }
  // Ended, or ended and only waiting for its parent to collect it.
  let stat;
  try { stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8'); } catch (e) { if (e.code === 'ENOENT') return true; throw e; }
  // The kernel keeps the first 15 bytes of a program's name.
  const name = stat.slice(stat.indexOf('(') + 1, stat.lastIndexOf(')'));
  return /^[ZX]/.test(stat.slice(stat.lastIndexOf(')') + 1).trimStart()) || (image !== undefined && name !== image.slice(0, 15));
}

const powershell = {
  name: 'PowerShell',
  started: tail => tail.includes('PS '),
  ok: "Write-Output ('ROCKET_' + 'TERMINAL_OK')",
  set: '$global:RocketProbe = 42',
  get: "Write-Output ('ROCKET_VALUE_' + $global:RocketProbe)",
  location: '(Get-Location).Path',
  accent: '[Console]::WriteLine([char]0x00E9)',
  sleep: 'Start-Sleep -Seconds 30',
  after: "Write-Output ('AFTER_' + 'INTERRUPT')",
  flood: "1..1200 | ForEach-Object { [Console]::WriteLine('ROCKET_OUTPUT_BOUND') }",
  error: "Write-Error 'ROCKET_EXPECTED_ERROR'",
  errorShown: 'WriteErrorException',
  variable: '$global:RocketProbe',
  edit: file => `[IO.File]::WriteAllText('${file.replaceAll("'", "''")}', 'ROCKET_TERMINAL_EDIT')`,
  denied: 'Get-ChildItem C:/',
  // The keyboard probes' lines (typed() in crates/rocket-mcp/src/terminal_probes.rs).
  historyFirst: "Write-Output 'ROCKET_HISTORY_FIRST'",
  completed: 'Write-Output',
  interruptVariable: '$global:',
};
const bash = {
  name: 'bash',
  started: tail => tail.trimEnd().endsWith('\n>'),
  ok: "printf 'ROCKET_%s\\n' TERMINAL_OK",
  set: 'RocketProbe=42',
  get: `printf 'ROCKET_VALUE_%s\\n' "$RocketProbe"`,
  location: 'pwd',
  accent: "printf '\\303\\251\\n'",
  sleep: 'sleep 30',
  after: "printf 'AFTER_%s\\n' INTERRUPT",
  flood: 'for n in $(seq 1200); do echo ROCKET_OUTPUT_BOUND; done',
  error: "printf 'ROCKET_EXPECTED_%s\\n' ERROR >&2",
  errorShown: 'ROCKET_EXPECTED_ERROR',
  variable: 'RocketProbe=',
  edit: file => `printf %s ROCKET_TERMINAL_EDIT > '${file.replaceAll("'", "'\\''")}'`,
  denied: 'ls /',
  historyFirst: "printf '%s\\n' ROCKET_HISTORY_FIRST",
  completed: 'echo',
  interruptVariable: 'RocketInterruptState',
};

module.exports = { windows, exe, gone, shell: windows ? powershell : bash };
