param([string]$Exe, [string]$Out)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Name D -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[W.D]::SetProcessDPIAware() | Out-Null
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9333"
$p = Start-Process $Exe -PassThru
Start-Sleep 15
node "$PSScriptRoot\eval.mjs" "JSON.stringify({media: document.getElementById('media').hidden ? 'hidden' : document.getElementById('media').title, weather: document.getElementById('weather').hidden ? 'hidden' : document.getElementById('weather').title})"
$s = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
foreach ($part in @(@{n='left'; x=0}, @{n='right'; x=$s.Width - 700})) {
  $bmp = New-Object System.Drawing.Bitmap 700, 48
  [System.Drawing.Graphics]::FromImage($bmp).CopyFromScreen($part.x, $s.Bottom - 48, 0, 0, $bmp.Size)
  $bmp.Save("$Out-$($part.n).png")
}
Stop-Process -Id $p.Id -Force
