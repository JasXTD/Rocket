# Sends normal Close to a wizard owned by the acceptance runner; never terminates a process.
param([Parameter(Mandatory)][int]$SetupProcessId)
$ErrorActionPreference='Stop'
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public static class RocketSetupWindow {
  public delegate bool WindowVisitor(IntPtr hwnd, IntPtr state);
  [DllImport("user32.dll")] static extern bool EnumWindows(WindowVisitor visit, IntPtr state);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wp, IntPtr lp);
  public static int Close(int[] processes) {
    var owned = new HashSet<int>(processes); int count = 0;
    EnumWindows((h, s) => { uint pid; GetWindowThreadProcessId(h, out pid);
      if (owned.Contains((int)pid) && IsWindowVisible(h)) {
        // Answer only this owned wizard's normal Exit Setup confirmation.
        var yes = GetDlgItem(h, 6);
        if (yes != IntPtr.Zero && IsWindowVisible(yes)) {
          if (PostMessage(h, 0x111, new IntPtr(6), IntPtr.Zero)) count++;
        } else if (PostMessage(h, 0x10, IntPtr.Zero, IntPtr.Zero)) count++;
      } return true;
    }, IntPtr.Zero); return count;
  }
}
'@
$end=(Get-Date).AddSeconds(20)
$sent=$false
do {
    $processes=@(Get-CimInstance Win32_Process)
    if($sent -and -not ($processes | Where-Object ProcessId -eq $SetupProcessId)){Write-Output 'Owned setup wizard cancelled.'; exit 0}
    $owned=[Collections.Generic.HashSet[int]]::new(); [void]$owned.Add($SetupProcessId)
    do {
        $added=$false
        foreach($process in $processes){if($owned.Contains([int]$process.ParentProcessId) -and $owned.Add([int]$process.ProcessId)){$added=$true}}
    } while($added)
    $closed=[RocketSetupWindow]::Close([int[]]@($owned))
    if($closed -gt 0){$sent=$true}
    Start-Sleep -Milliseconds 100
} while((Get-Date) -lt $end)
throw 'No owned visible setup wizard appeared.'
