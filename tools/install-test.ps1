param([string]$Setup)
Add-Type -Name U -Namespace W -MemberDefinition '[DllImport("user32.dll")] public static extern System.IntPtr FindWindow(string c, string n); [DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);'
function Taskbar { [W.U]::IsWindowVisible([W.U]::FindWindow("Shell_TrayWnd", $null)) }
$dir = "$env:LOCALAPPDATA\Pane"
$key = Get-ChildItem HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall | Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq "Pane" }

Start-Process $Setup -ArgumentList "/S" -Wait
$key = Get-ChildItem HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall | Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq "Pane" }
"installed: folder={0} exe={1} in-Installed-apps={2} start-menu={3}" -f (Test-Path $dir), (Test-Path "$dir\pane.exe"), [bool]$key, [bool](Get-ChildItem "$env:APPDATA\Microsoft\Windows\Start Menu\Programs" -Recurse -Filter "Pane*.lnk")

Start-Process "$dir\pane.exe"
Start-Sleep 5
"running: taskbar visible={0}, pane processes={1}" -f (Taskbar), @(Get-Process pane -ErrorAction SilentlyContinue).Count

$un = (Get-ItemProperty $key.PSPath).UninstallString.Trim('"')
Start-Process $un -ArgumentList "/S" -Wait
Start-Sleep 4  # the NSIS uninstaller copies itself to %TEMP% and returns early
"uninstalled: taskbar visible={0}, pane processes={1}, folder left={2}, in-Installed-apps={3}, state file left={4}" -f (Taskbar), @(Get-Process pane -ErrorAction SilentlyContinue).Count, (Test-Path $dir), [bool](Get-ChildItem HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall | Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq "Pane" }), (Test-Path "$env:LOCALAPPDATA\org.gruening.pane\taskbar-autohide.txt")
