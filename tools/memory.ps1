# Lists Pane's processes (pane.exe + its WebView2 processes) with type, working set and private bytes.
# Usage: tools/memory.ps1            (whatever Pane is running now)
$rows = @()
foreach ($p in Get-CimInstance Win32_Process -Filter "Name='pane.exe'") {
  $rows += [pscustomobject]@{ Pid = $p.ProcessId; Type = 'pane'; WS = $p.WorkingSetSize; Private = $p.PrivatePageCount }
}
foreach ($p in Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" | Where-Object { $_.CommandLine -match 'org.gruening.pane' }) {
  $type = if ($p.CommandLine -match '--type=([\w-]+)') { $Matches[1] } else { 'browser' }
  if ($p.CommandLine -match '--utility-sub-type=([\w.]+)') { $type += ' ' + ($Matches[1] -replace '.*\.', '') }
  $rows += [pscustomobject]@{ Pid = $p.ProcessId; Type = $type; WS = $p.WorkingSetSize; Private = $p.PrivatePageCount }
}
$rows | Sort-Object Type | Format-Table Pid, Type, @{n='WS MB'; e={'{0:N1}' -f ($_.WS / 1MB)}; a='right'}, @{n='Private MB'; e={'{0:N1}' -f ($_.Private / 1MB)}; a='right'} -AutoSize
$web = $rows | Where-Object Type -ne 'pane'
'pane: WS {0:N0} MB, private {1:N0} MB | webview ({2} procs): WS {3:N0} MB, private {4:N0} MB | total private {5:N0} MB' -f `
  (($rows | Where-Object Type -eq 'pane' | Measure-Object WS -Sum).Sum / 1MB), (($rows | Where-Object Type -eq 'pane' | Measure-Object Private -Sum).Sum / 1MB),
  @($web).Count, (($web | Measure-Object WS -Sum).Sum / 1MB), (($web | Measure-Object Private -Sum).Sum / 1MB), (($rows | Measure-Object Private -Sum).Sum / 1MB)
