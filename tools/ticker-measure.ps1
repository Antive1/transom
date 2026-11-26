# Izolovane meranie posuvneho textu: spusti lištu s DLHYM simulovanym nazvom (--sim-media, len test vzhladu) a
# --ticker-stats, po rozbehu zmeria CPU (cpu-sample) a vypise posledny suhrn plynulosti z logu.
# Nikdy nesiaha na skutocnu konfiguraciu ani prehravanie: TRANSOM_CONFIG/TRANSOM_DATA idu do docasneho adresara.
# Pouzitie: ticker-measure.ps1 -Speed normal -Secs 30 [-Title "..."] [-Paused] [-Short] [-Exe cesta] [-Label text]
param([ValidateSet("slow", "normal", "fast")][string]$Speed = "normal", [int]$Secs = 30, [string]$Title = "",
      [switch]$Paused, [switch]$Short, [switch]$Off, [string]$Exe = (Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\transom-app.exe"),
      [string]$Label = "")
$root = Join-Path $env:TEMP "transom-ticker-measure"
Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$root\data" | Out-Null
if (-not $Title) { $Title = "Example Artist - Example Song (Long Remix) [feat. Example Guest] - a very long title that overflows every bar " * 2 }
if ($Short) { $Title = "Short song" }
$marq = if ($Off) { "false" } else { "true" }
$cfg = '{"version":5,"media":{"marquee":' + $marq + ',"marquee_speed":"' + $Speed + '"}}'
[IO.File]::WriteAllText("$root\config.json", $cfg, (New-Object Text.UTF8Encoding($false)))
$env:TRANSOM_CONFIG = "$root\config.json"; $env:TRANSOM_DATA = "$root\data"
$sim = $Title + $(if ($Paused) { "|paused" } else { "" })
$p = Start-Process -FilePath $Exe -ArgumentList "--no-tray", "--no-media", "--ticker-stats", "--secs", ($Secs + 14), "--sim-media", "`"$sim`"", "--log", "`"$root\app.log`"" -PassThru
Start-Sleep -Seconds 6
$p.Refresh(); $t0 = $p.TotalProcessorTime; $w0 = Get-Date
Start-Sleep -Seconds $Secs
$p.Refresh()
$cpu = ($p.TotalProcessorTime - $t0).TotalMilliseconds; $wall = ((Get-Date) - $w0).TotalMilliseconds
$paints = (Select-String -Path "$root\app.log" -Pattern "paint #").Count
"{0} speed={1}: cpu={2:N1} ms / {3:N1} s = {4:N3} % jadra; WS={5:N1} MB; paint-riadkov v logu={6}" -f $Label, $Speed, $cpu, ($wall / 1000), (100 * $cpu / $wall), ($p.WorkingSet64 / 1MB), $paints
Start-Sleep -Seconds 1
$last = Select-String -Path "$root\app.log" -Pattern "plynulost posuvu" | Select-Object -Last 1
if ($last) { $last.Line } else { "(ziadny suhrn plynulosti)" }
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 1
