# Nezavisle vypocet dennych sucтov tokenov z lokalnych prepisov (Claude Code, Codex) na porovnanie s `history_dump`.
# SAMOSTATNA implementacia (PowerShell, ConvertFrom-Json, dedup po kluci) - nepouziva kod transom. Subory sa len CITAJU,
# nevypisuje sa ziadny obsah rozhovorov, len cisla a datumy. Lokalny den = casove pasmo Windows (TimeZoneInfo.Local).
param([int]$Year = (Get-Date).Year, [int]$Month = (Get-Date).Month)
$ErrorActionPreference = "Stop"
$home_ = $env:USERPROFILE
$tz = [TimeZoneInfo]::Local
function LocalDay([string]$iso) {
  $dt = [DateTimeOffset]::Parse($iso, [Globalization.CultureInfo]::InvariantCulture, [Globalization.DateTimeStyles]::AssumeUniversal)
  [TimeZoneInfo]::ConvertTime($dt, $tz).ToString("yyyy-MM-dd")
}
# citanie so zdielanym pristupom (aktivna relacia je otvorena zapisom inym procesom); len na citanie
function ReadShared([string]$path) {
  $fs = New-Object IO.FileStream($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, ([IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete))
  $sr = New-Object IO.StreamReader($fs, [Text.Encoding]::UTF8)
  try { while (($l = $sr.ReadLine()) -ne $null) { $l } } finally { $sr.Dispose(); $fs.Dispose() }
}
$claude = @{}; $codex = @{}
# --- Claude Code: assistant riadky s message.usage; tokeny = input + output + cache_read + cache_creation; dedup (message.id|requestId)
$seen = New-Object 'System.Collections.Generic.HashSet[string]'
Get-ChildItem (Join-Path $home_ ".claude\projects") -Recurse -Filter *.jsonl -ErrorAction SilentlyContinue | ForEach-Object {
  foreach ($line in (ReadShared $_.FullName)) {
    if (-not $line.Contains('"usage"') -or -not $line.Contains('"assistant"')) { continue }
    try { $o = $line | ConvertFrom-Json } catch { continue }
    if ($o.type -ne "assistant" -or -not $o.message.usage -or -not $o.timestamp) { continue }
    $u = $o.message.usage
    $t = [int64]($u.input_tokens + $u.output_tokens + $u.cache_read_input_tokens + $u.cache_creation_input_tokens)
    if ($t -eq 0) { continue }
    $key = "$($o.message.id)|$($o.requestId)"
    if ($key -eq "|") { $key = "$($o.timestamp):$($u.input_tokens):$($u.output_tokens)" }
    if (-not $seen.Add($key)) { continue }
    $d = LocalDay $o.timestamp
    if ($claude.ContainsKey($d)) { $claude[$d] += $t } else { $claude[$d] = $t }
  }
}
# --- Codex: token_usage_record; tokeny = input_tokens (obsahuje cachovany vstup) + output_tokens; dedup response_id
$seen2 = New-Object 'System.Collections.Generic.HashSet[string]'
Get-ChildItem (Join-Path $home_ ".codex\sessions") -Recurse -Filter *.jsonl -ErrorAction SilentlyContinue | ForEach-Object {
  foreach ($line in (ReadShared $_.FullName)) {
    if (-not $line.Contains('token_usage_record')) { continue }
    try { $o = $line | ConvertFrom-Json } catch { continue }
    if ($o.type -ne "token_usage_record" -or -not $o.payload.usage -or -not $o.timestamp) { continue }
    $u = $o.payload.usage
    $key = [string]$o.payload.response_id
    if (-not $key) { $key = "$($o.timestamp):$($u.input_tokens):$($u.output_tokens)" }
    if (-not $seen2.Add($key)) { continue }
    $t = [int64]($u.input_tokens + $u.output_tokens)
    $d = LocalDay $o.timestamp
    if ($codex.ContainsKey($d)) { $codex[$d] += $t } else { $codex[$d] = $t }
  }
}
"date`tclaude_tokens`tcodex_tokens`tall_tokens"
$n = [DateTime]::DaysInMonth($Year, $Month)
for ($d = 1; $d -le $n; $d++) {
  $k = "{0}-{1:00}-{2:00}" -f $Year, $Month, $d
  $c = if ($claude.ContainsKey($k)) { $claude[$k] } else { 0 }
  $x = if ($codex.ContainsKey($k)) { $codex[$k] } else { 0 }
  "{0}`t{1}`t{2}`t{3}" -f $k, $c, $x, ($c + $x)
}
