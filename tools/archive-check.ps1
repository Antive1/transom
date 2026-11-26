# Overenie archivu dennych suctov. Dve casti:
#  A) SKUTOCNE lokalne logy (len CITANIE; archiv ide do docasneho suboru, nie do dat transom): mesacne sucty s archivom
#     sa porovnaju s nezavislym vypoctom (tools/history-check.ps1); druhy beh nic nepripocita (subor sa nezmeni).
#  B) SYNTETICKE logy v docasnom profile (USERPROFILE) - ziadne skutocne data: po zmazani najstarsieho logu archiv drzi
#     povodne sucty (rovnake ako pred zmazanim), bez archivu dni zmiznu.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$dump = Join-Path $root "target\debug\examples\history_dump.exe"
$check = Join-Path $root "tools\history-check.ps1"
$now = Get-Date; $Y = $now.Year; $M = $now.Month
# `history_dump`: date, stav, claude, codex, all, ...; `history-check.ps1`: date, claude, codex, all (posun o 1 stlpec)
function Parse($lines, [int]$o = 2) {
  $lines | Where-Object { $_ -match '^\d{4}-' } | ForEach-Object { $f = $_ -split "`t"; [pscustomobject]@{ date = $f[0]; claude = [int64]$f[$o]; codex = [int64]$f[$o + 1]; all = [int64]$f[$o + 2] } }
}
function Compare-Days($a, $b, $label) {
  $bad = 0
  foreach ($x in $a) { $y = $b | Where-Object date -eq $x.date; if ($y.claude -ne $x.claude -or $y.codex -ne $x.codex -or $y.all -ne $x.all) { $bad++; Write-Host ("  ROZDIEL {0}: {1}/{2} vs {3}/{4}" -f $x.date, $x.claude, $x.codex, $y.claude, $y.codex) } }
  Write-Host ("{0}: dni porovnanych: {1}, rozdielov: {2}" -f $label, @($a).Count, $bad)
  return $bad
}

"=== A) skutocne logy (len citanie), archiv v docasnom subore ==="
$arch = Join-Path $env:TEMP "transom-archive-real-check.json"
Remove-Item $arch -ErrorAction SilentlyContinue
$run1 = Parse (& $dump --archive $arch $Y $M)
$h1 = (Get-FileHash $arch).Hash
$run2 = Parse (& $dump --archive $arch $Y $M)
$h2 = (Get-FileHash $arch).Hash
$ind = Parse (powershell -NoProfile -ExecutionPolicy Bypass -File $check -Year $Y -Month $M) 1
$bad = 0
$bad += Compare-Days $run1 $ind "transom (s archivom) vs nezavisly vypocet"
$bad += Compare-Days $run1 $run2 "prvy vs druhy beh (idempotencia)"
"subor archivu po 1. a 2. behu rovnaky: $($h1 -eq $h2) (informativne: zivá relacia medzi behmi pridava tokeny; idempotencia sa dokazuje v casti B a porovnanim dni)"
$txt = Get-Content $arch -Raw
"archiv: {0} B, riadkov dna: {1}; obsahuje cesty/texty konverzacii: {2}" -f $txt.Length, ([regex]::Matches($txt, '"model"')).Count, ($txt -match 'prompt|content|cwd|\\\\Users|projects')
"mesiac spolu: transom {0} vs nezavisly {1}" -f ($run1 | Measure-Object all -Sum).Sum, ($ind | Measure-Object all -Sum).Sum
Remove-Item $arch -ErrorAction SilentlyContinue

"=== B) syntetické logy: zmazanie najstarsieho logu ==="
$prof = Join-Path $env:TEMP "transom-archive-synth"
Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue
$dir = Join-Path $prof ".claude\projects\p"
New-Item -ItemType Directory -Force $dir | Out-Null
function Line($day, $id, $inp) { $ts = "{0}-{1:00}-{2:00}T10:00:00.000Z" -f $Y, $M, $day; '{"type":"assistant","timestamp":"' + $ts + '","requestId":"r-' + $id + '","message":{"id":"m-' + $id + '","model":"claude-opus-5-5","usage":{"input_tokens":' + $inp + ',"output_tokens":10,"cache_read_input_tokens":' + ($inp * 3) + ',"cache_creation_input_tokens":0}}}' }
$old = (2..5 | ForEach-Object { Line $_ "o$_" (1000 * $_) }) -join "`n"
$new = (8..10 | ForEach-Object { Line $_ "n$_" (700 * $_) }) -join "`n"
[IO.File]::WriteAllText("$dir\old.jsonl", $old + "`n"); [IO.File]::WriteAllText("$dir\new.jsonl", $new + "`n")
$saveProfile = $env:USERPROFILE; $env:USERPROFILE = $prof
try {
  $arch2 = Join-Path $prof "archive.json"
  $before = Parse (& $dump --archive $arch2 $Y $M)
  $indBefore = Parse (powershell -NoProfile -ExecutionPolicy Bypass -File $check -Year $Y -Month $M) 1
  $bad += Compare-Days $before $indBefore "pred zmazanim: transom vs nezavisly"
  # idempotencia na syntetickych datach: opakovany beh nezmeni subor archivu ani ziadny den
  $hashA = (Get-FileHash $arch2).Hash
  $again = Parse (& $dump --archive $arch2 $Y $M)
  $bad += Compare-Days $before $again "opakovany beh (idempotencia)"
  $same = ((Get-FileHash $arch2).Hash -eq $hashA)
  Write-Host "subor archivu po opakovanom behu rovnaky: $same"
  if (-not $same) { $bad++ }
  Remove-Item "$dir\old.jsonl"        # Claude Code zmazal stary prepis
  $afterWith = Parse (& $dump --archive $arch2 $Y $M)
  $afterWithout = Parse (& $dump $Y $M)
  $bad += Compare-Days $before $afterWith "po zmazani logu S archivom: rovnake ako pred zmazanim"
  $lost = ($before | Measure-Object all -Sum).Sum - ($afterWithout | Measure-Object all -Sum).Sum
  "po zmazani logu BEZ archivu sa stratilo tokenov: $lost (mesiac spolu {0} -> {1})" -f ($before | Measure-Object all -Sum).Sum, ($afterWithout | Measure-Object all -Sum).Sum
  "dni 2-5 s archivom: " + (($afterWith | Where-Object { [int]($_.date.Substring(8)) -in 2..5 } | ForEach-Object { $_.all }) -join ", ")
  "dni 2-5 bez archivu: " + (($afterWithout | Where-Object { [int]($_.date.Substring(8)) -in 2..5 } | ForEach-Object { $_.all }) -join ", ")
} finally { $env:USERPROFILE = $saveProfile; Remove-Item $prof -Recurse -Force -ErrorAction SilentlyContinue }
if ($bad -eq 0) { "VYSLEDOK: vsetko sedi" } else { "VYSLEDOK: ROZDIELY ($bad)"; exit 1 }
