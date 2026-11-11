param([string]$Exe)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string c, string n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct APPBARDATA { public int cbSize; public IntPtr hWnd; public uint msg; public uint edge; public int l,t,r,b; public IntPtr lParam; }
  [DllImport("shell32.dll")] public static extern UIntPtr SHAppBarMessage(uint m, ref APPBARDATA d);
  public static uint AutoHide() { var d = new APPBARDATA(); d.cbSize = Marshal.SizeOf(d); return (uint)SHAppBarMessage(4, ref d); }
}
"@
$state = "$env:LOCALAPPDATA\org.gruening.pane\taskbar-autohide.txt"
function Report($label) {
  $tray = [W]::FindWindow("Shell_TrayWnd", $null)
  $procs = Get-Process pane -ErrorAction SilentlyContinue
  $mb = ($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB
  $wv = (Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'org.gruening.pane' } | ForEach-Object { (Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue).WorkingSet64 } | Measure-Object -Sum).Sum / 1MB
  "{0,-16} taskbar visible={1}  autohide-state={2}  state-file={3}  pane procs={4} ({5:N0} MB)  webview={6:N0} MB" -f $label, [W]::IsWindowVisible($tray), [W]::AutoHide(), (Test-Path $state), @($procs).Count, $mb, $wv
}
Report "before"
$p = Start-Process $Exe -PassThru
Start-Sleep 6
Report "pane running"
$cpu1 = (Get-Process -Id $p.Id).CPU; Start-Sleep 5; $cpu2 = (Get-Process -Id $p.Id).CPU
"idle CPU over 5 s: {0:N3} s of CPU time" -f ($cpu2 - $cpu1)
Stop-Process -Id $p.Id -Force   # like "End task" in Task Manager
Start-Sleep 3
Report "after kill"
