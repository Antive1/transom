# SKUTOCNY test mysou panela AI (Left/Used, zalozka Month, den v grafe, koliesko). Pouziva SetCursorPos + mouse_event
# (skutocny vstup na ploche), preto:
#  - spusta IZOLOVANU instanciu transom (kopia konfiguracie v TEMP, data v TEMP; skutocny config.json sa nemeni),
#  - PRED KAZDYM klikom over, ze okno pod kurzorom je popover (trieda TransomPopover) procesu TEJTO instancie - inak sa
#    nekliká a test sa preruší,
#  - po teste vrati kurzor na povodne miesto. Nemeni rozlisenie, DPI, cas ani nastavenia systemu.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices; using System.Text;
public static class M {
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, int data, UIntPtr extra);
  [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
  [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
}
"@
function Under($x, $y) {
  $p = New-Object M+POINT; $p.X = $x; $p.Y = $y
  $h = [M]::GetAncestor([M]::WindowFromPoint($p), 2)
  $pid_ = 0; [void][M]::GetWindowThreadProcessId($h, [ref]$pid_)
  $sb = New-Object Text.StringBuilder 64; [void][M]::GetClassName($h, $sb, 64)
  [pscustomobject]@{ Class = $sb.ToString(); Pid = $pid_ }
}
$script:aborted = $false
function Click($name, $xy, $app) {
  if ($xy -eq "-" -or -not $xy) { Write-Host "  [$name] cielovy prvok nie je viditelny - preskocene"; return $false }
  $x, $y = [int[]]($xy -split ","); $u = Under $x $y
  if ($u.Class -ne "TransomPopover" -or $u.Pid -ne $app.Id) {
    Write-Host "  [$name] POD KURZOROM NIE JE POPOVER TEJTO INSTANCIE (trieda=$($u.Class) pid=$($u.Pid)) - NEKLIKAM, test sa prerusuje"
    $script:aborted = $true; return $false
  }
  [void][M]::SetCursorPos($x, $y); Start-Sleep -Milliseconds 250
  [M]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 80
  [M]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)
  Write-Host "  [$name] kliknute na $x,$y (pod kurzorom: $($u.Class))"
  Start-Sleep -Milliseconds 1100; return $true
}
function Last($log) { (Get-Content $log | Where-Object { $_ -match "real-targets" } | Select-Object -Last 1) }
function Field($line, $key) { if ($line -match "(?:^|\s)$key=([^ ]+)") { $Matches[1] } else { "" } }

$root = Join-Path $env:TEMP "transom-live-mouse"
Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$root\data" | Out-Null
$cfg = Get-Content "$env:APPDATA\transom\config.json" -Raw | ConvertFrom-Json
$cfg.usage.ring_mode = "used"
[IO.File]::WriteAllText("$root\config.json", ($cfg | ConvertTo-Json -Depth 12), (New-Object Text.UTF8Encoding($false)))
$real = Join-Path $env:LOCALAPPDATA "transom\claude-statusline.jsonl"
if (Test-Path $real) { Copy-Item $real "$root\data\claude-statusline.jsonl" }
$env:TRANSOM_CONFIG = "$root\config.json"; $env:TRANSOM_DATA = "$root\data"
$exe = Join-Path (Split-Path -Parent $PSScriptRoot) "target\debug\transom-app.exe"
$log = "$root\run.log"
$orig = New-Object M+POINT; [void][M]::GetCursorPos([ref]$orig)
$app = Start-Process $exe -ArgumentList @("--no-tray", "--no-media", "--test-ai-real", "--secs", "75", "--log", "`"$log`"") -PassThru
try {
  # pockat na otvorenie panela a nacitanie dat
  $line = $null
  for ($i = 0; $i -lt 60; $i++) { Start-Sleep -Milliseconds 500; $line = Last $log; if ($line -and (Field $line "mode_left") -ne "-" -and (Field $line "mode_left") -ne "") { break } }
  if (-not $line) { throw "panel sa neotvoril" }
  Write-Host "Panel otvoreny. Pociatocny stav: mode=$(Field $line 'mode') kruh=$(($line -replace '.*ring=''([^'']*)''.*','$1'))"
  $before = $line
  # 1) Left (zostava) -> karty aj kruh v liste sa zmenia hned
  if (Click "Left" (Field $line "mode_left") $app) {
    $line = Last $log; Write-Host "  po kliku na Left: mode=$(Field $line 'mode') kruh=$(($line -replace '.*ring=''([^'']*)''.*','$1'))"
    $shotA = New-Object System.Drawing.Bitmap 680, 1180; $g = [System.Drawing.Graphics]::FromImage($shotA); $g.CopyFromScreen(0, 0, 0, 0, $shotA.Size); $shotA.Save("$root\after-left.png"); $g.Dispose()
    $sw = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds.Width
    $b2 = New-Object System.Drawing.Bitmap 520, 36; $g2 = [System.Drawing.Graphics]::FromImage($b2); $g2.CopyFromScreen($sw - 520, 0, 0, 0, $b2.Size); $b2.Save("$root\after-left-bar.png"); $g2.Dispose()
  }
  # 2) Used
  if (-not $script:aborted) { $line = Last $log; if (Click "Used" (Field $line "mode_used") $app) { $line = Last $log; Write-Host "  po kliku na Used: mode=$(Field $line 'mode') kruh=$(($line -replace '.*ring=''([^'']*)''.*','$1'))" } }
  # 3) Month
  if (-not $script:aborted) { $line = Last $log; if (Click "Month" (Field $line "tab_month") $app) { $line = Last $log; Write-Host "  po kliku na Month: tab=$(Field $line 'month_ui')" } }
  # 4) den v grafe (dnesok)
  if (-not $script:aborted) { $line = Last $log; if (Click "Day(dnes)" (Field $line "day") $app) { $line = Last $log; Write-Host "  po kliku na den: month_ui=$(Field $line 'month_ui')"; $s = New-Object System.Drawing.Bitmap 680, 1180; $g = [System.Drawing.Graphics]::FromImage($s); $g.CopyFromScreen(0, 0, 0, 0, $s.Size); $s.Save("$root\after-day.png"); $g.Dispose() } }
  # 5) jednotka (odhad ceny)
  if (-not $script:aborted) { $line = Last $log; Click "Est. cost" (Field $line "unit_cost") $app | Out-Null }
  # 6) koliesko nad panelom (posun)
  if (-not $script:aborted) {
    $line = Last $log; $xy = Field $line "tab_month"
    if ($xy -and $xy -ne "-") {
      $x, $y = [int[]]($xy -split ","); $u = Under ($x - 20) ($y + 20)
      if ($u.Class -eq "TransomPopover" -and $u.Pid -eq $app.Id) {
        [void][M]::SetCursorPos($x - 20, $y + 20); Start-Sleep -Milliseconds 250
        [M]::mouse_event(0x0800, 0, 0, -360, [UIntPtr]::Zero); Start-Sleep -Milliseconds 1200
        $line = Last $log; Write-Host "  po kolieska (3 zuby dole): posun=$(Field $line 'scroll')"
      } else { Write-Host "  [koliesko] pod kurzorom nie je popover - preskocene" }
    }
  }
  $cfgNow = (Get-Content "$root\config.json" -Raw | ConvertFrom-Json).usage.ring_mode
  Write-Host "Konfiguracia v izolovanom subore po teste: ring_mode = $cfgNow"
  Write-Host ("Skutocny config.json pouzivatela (nezmeneny): ring_mode = " + (Get-Content "$env:APPDATA\transom\config.json" -Raw | ConvertFrom-Json).usage.ring_mode)
  if ($script:aborted) { Write-Host "TEST PRERUSENY (bezpecnostna kontrola)" } else { Write-Host "TEST DOBEHOL" }
} finally {
  [void][M]::SetCursorPos($orig.X, $orig.Y)
  if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
}
