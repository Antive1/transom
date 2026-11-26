# Nasnima okno podla triedy (napr. TransomSettings) do PNG a vypise jeho obdlznik vo fyzickych px.
# Pouzitie: shot-window.ps1 -Class TransomSettings -Out <subor.png>
param([string]$Class = "TransomSettings", [string]$Out = "$env:TEMP\transom-window.png")
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class SW {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
  [DllImport("dwmapi.dll")] public static extern int DwmGetWindowAttribute(IntPtr h, int a, out RECT r, int s);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
'@
[void][SW]::SetProcessDpiAwarenessContext([IntPtr](-4))
$h = [SW]::FindWindow($Class, [NullString]::Value)
if ($h -eq [IntPtr]::Zero) { "okno $Class nenajdene"; return }
$r = New-Object SW+RECT
[void][SW]::DwmGetWindowAttribute($h, 9, [ref]$r, 16)   # DWMWA_EXTENDED_FRAME_BOUNDS
$w = $r.R - $r.L; $hh = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $hh
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.L, $r.T, 0, 0, (New-Object System.Drawing.Size $w, $hh))
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png); $g.Dispose(); $bmp.Dispose()
"$Out rect=($($r.L),$($r.T))-($($r.R),$($r.B))"
