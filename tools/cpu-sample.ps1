# Zmeria CPU procesu pocas N sekund: rozdiel TotalProcessorTime / uplynuly cas (v % jedneho jadra), plus pracovnu
# pamat a pocet handlov. Pouzitie: cpu-sample.ps1 -Name transom-app -Seconds 30 [-Label "marquee playing"]
param([string]$Name = "transom-app", [int]$Seconds = 30, [string]$Label = "")
$p = Get-Process -Name $Name -ErrorAction Stop | Select-Object -First 1
$t0 = $p.TotalProcessorTime; $w0 = Get-Date
Start-Sleep -Seconds $Seconds
$p.Refresh()
$cpu = ($p.TotalProcessorTime - $t0).TotalMilliseconds
$wall = ((Get-Date) - $w0).TotalMilliseconds
"{0}: cpu={1:N1} ms za {2:N1} s = {3:N3} % jadra; WS={4:N1} MB private={5:N1} MB handles={6}" -f `
    $Label, $cpu, ($wall / 1000), (100 * $cpu / $wall), ($p.WorkingSet64 / 1MB), ($p.PrivateMemorySize64 / 1MB), $p.HandleCount
