param([string]$Exe, [string]$Out)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class T {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
  public static void Click(int x, int y) { SetCursorPos(x, y); mouse_event(2,0,0,0,UIntPtr.Zero); mouse_event(4,0,0,0,UIntPtr.Zero); }
}
"@
[T]::SetProcessDPIAware() | Out-Null
function Shot($name, $x, $y, $w, $h) {
  $bmp = New-Object System.Drawing.Bitmap $w, $h
  [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($x, $y, 0, 0, $bmp.Size)
  $bmp.Save("$Out-$name.png")
}
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9333"
$p = Start-Process $Exe -PassThru
Start-Sleep 9
$info = node "$PSScriptRoot\eval.mjs" "JSON.stringify({tray: document.querySelectorAll('#tray .tray-icon').length, tips: [...document.querySelectorAll('#tray .tray-icon')].map(b=>b.title.split('\n')[0]), arrowHidden: document.getElementById('overflow').hidden, arrow: document.getElementById('overflow').getBoundingClientRect().toJSON()})" | ConvertFrom-Json
"bar tray icons: $($info.tray) $($info.tips -join ' | ')   arrow hidden=$($info.arrowHidden)"
$screen = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
Shot "bar" ($screen.Width - 900) ($screen.Bottom - 48) 900 48
if (-not $info.arrowHidden) {
  $ax = [int]($info.arrow.left + $info.arrow.width / 2); $ay = $screen.Bottom - 48 + [int]($info.arrow.top + $info.arrow.height / 2)
  [T]::Click($ax, $ay); Start-Sleep 1.5
  Shot "popup" ($ax - 200) ($screen.Bottom - 260) 400 260
  "clicked arrow at $ax,$ay"
}
Stop-Process -Id $p.Id -Force
Start-Sleep 4
Shot "explorer" ($screen.Width - 700) ($screen.Bottom - 48) 700 48
