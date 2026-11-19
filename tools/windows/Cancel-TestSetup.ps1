# Sends normal Close to a wizard owned by the acceptance runner; never terminates a process.
param([Parameter(Mandatory)][int]$SetupProcessId)
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class RocketSetupWindow {
  public delegate bool WindowVisitor(IntPtr hwnd, IntPtr state);
  [DllImport("user32.dll")] static extern bool EnumWindows(WindowVisitor visit, IntPtr state);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] static extern bool IsWindowEnabled(IntPtr hwnd);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr hwnd, StringBuilder name, int capacity);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wp, IntPtr lp);
  public static bool Close(int[] processes, bool requested) {
    var owned = new HashSet<int>(processes);
    EnumWindows((h, s) => { uint pid; GetWindowThreadProcessId(h, out pid);
      if (owned.Contains((int)pid) && IsWindowVisible(h) && IsWindowEnabled(h)) {
        var name = new StringBuilder(128);
        GetClassName(h, name, name.Capacity);
        // Answer only this owned wizard's normal Exit Setup confirmation.
        var yes = GetDlgItem(h, 6);
        if (requested && name.ToString() == "#32770" && yes != IntPtr.Zero && IsWindowVisible(yes)) {
          PostMessage(h, 0x111, new IntPtr(6), yes);
        } else if (!requested && name.ToString() == "TWizardForm") {
          requested = PostMessage(h, 0x10, IntPtr.Zero, IntPtr.Zero);
        }
      } return true;
    }, IntPtr.Zero); return requested;
  }
}
'@
$setup=Get-Process -Id $SetupProcessId -ErrorAction Stop
$end=(Get-Date).AddSeconds(20)
$sent=$false
do {
    if($setup.HasExited){break}
    $processes=@(Get-CimInstance Win32_Process -Property ProcessId,ParentProcessId)
    $owned=[Collections.Generic.HashSet[int]]::new(); [void]$owned.Add($SetupProcessId)
    do {
        $added=$false
        foreach($process in $processes){if($owned.Contains([int]$process.ParentProcessId) -and $owned.Add([int]$process.ProcessId)){$added=$true}}
    } while($added)
    $sent=[RocketSetupWindow]::Close([int[]]@($owned),$sent)
    if($setup.WaitForExit(100)){break}
} while((Get-Date) -lt $end)
if($sent -and $setup.HasExited){Write-Output 'Owned setup wizard cancelled.'; exit 0}
if($sent){throw 'The owned setup wizard received Close but did not exit.'}
throw 'No owned visible setup wizard appeared.'
