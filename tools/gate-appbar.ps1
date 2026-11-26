param([string]$Name = "app")
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices;
public class GW {
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT p);
 [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
 [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
 public static string TitleAt(int x, int y) { var h = WindowFromPoint(new POINT{X=x,Y=y}); var r = GetAncestor(h, 2); var sb = new StringBuilder(128); GetWindowText(r, sb, 128); return sb.ToString(); }
}
'@
[void][GW]::SetProcessDPIAware()
function Pump($s) { $e = (Get-Date).AddSeconds($s); while ((Get-Date) -lt $e) { [Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 50 } }
function WA { ([Windows.Forms.Screen]::AllScreens | Sort-Object { -not $_.Primary } | % { $_.WorkingArea.Y }) -join "/" }
$exe = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\transom-app.exe"
$res = [ordered]@{}
$p = Start-Process $exe -ArgumentList "--secs 60 --log `"$env:TEMP\s6\gate-$Name.log`"" -PassThru
Pump 4
$res.workarea_primary_vertical = WA   # ocakavane 28/-377
# maximalizovane okno
$m = New-Object Windows.Forms.Form; $m.WindowState = 'Maximized'; $m.Show(); $m.Activate(); Pump 1.5
$res.maximized_bounds = "$($m.Bounds)  (ocakavane Y=20 pri rame 8px => viditelne od 28)"
$m.Close(); Pump 0.5
# fullscreen: hide
$prim = [Windows.Forms.Screen]::AllScreens | Where-Object Primary
$res.bar_visible_before_fs = [GW]::TitleAt(300, 10)
$f = New-Object Windows.Forms.Form; $f.FormBorderStyle = 'None'; $f.StartPosition = 'Manual'; $f.Bounds = $prim.Bounds; $f.Show(); $f.Activate(); [void][GW]::SetForegroundWindow($f.Handle); Pump 2
$res.top_at_300_10_during_fs = "'" + [GW]::TitleAt(300, 10) + "'  (ocakavane: nie s6-bar)"
$f.Close(); Pump 1.5
$res.top_at_300_10_after_fs = [GW]::TitleAt(300, 10)
# zabitie procesu
Stop-Process -Id $p.Id -Force
$sw = [Diagnostics.Stopwatch]::StartNew()
while ((WA) -ne "0/-405" -and $sw.Elapsed.TotalSeconds -lt 8) { Start-Sleep -Milliseconds 100 }
$res.workarea_po_kill = (WA) + "  po " + [math]::Round($sw.Elapsed.TotalSeconds, 2) + " s  (ocakavane 0/-405)"
"=== $Name ==="
$res.GetEnumerator() | % { "{0,-30} {1}" -f $_.Key, $_.Value }
Get-Content "$env:TEMP\s6\gate-$Name.log" | Select-String "ABN_FULLSCREEN" | % { $_.Line.Substring(0, [Math]::Min(100, $_.Line.Length)) }
