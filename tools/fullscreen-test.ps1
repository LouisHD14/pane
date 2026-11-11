param([string]$Exe)
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public static class P {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  public static string FgTitle() { var t = new StringBuilder(100); GetWindowText(GetForegroundWindow(), t, 100); return t.ToString(); }
  public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2,0,0,0,UIntPtr.Zero); mouse_event(4,0,0,0,UIntPtr.Zero); }
  public static string Bars(uint pid) { var r = new List<string>(); EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); var t = new StringBuilder(10); GetWindowText(h, t, 10);
    if (p == pid && t.ToString() == "Pane") r.Add(IsWindowVisible(h) ? "shown" : "hidden"); return true; }, IntPtr.Zero); return String.Join(",", r); }
}
"@
[P]::SetProcessDPIAware() | Out-Null
$p = Start-Process $Exe -PassThru
Start-Sleep 6
"bars at start:            " + [P]::Bars($p.Id)
$f = New-Object System.Windows.Forms.Form
$f.Text = "FullscreenTest"; $f.FormBorderStyle = "None"; $f.StartPosition = "Manual"; $f.Bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; $f.BackColor = "Black"; $f.TopMost = $true
$f.Show(); [System.Windows.Forms.Application]::DoEvents(); Start-Sleep 0.5
[P]::Click(1280, 600)   # real click gives the test window the foreground
for ($i = 0; $i -lt 25; $i++) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
"bars with fullscreen app: " + [P]::Bars($p.Id) + "   foreground='" + [P]::FgTitle() + "'"
$f.Close(); [System.Windows.Forms.Application]::DoEvents()
Start-Sleep 2.5
"bars after it closed:     " + [P]::Bars($p.Id)
Stop-Process -Id $p.Id -Force
