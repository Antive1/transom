# Nasnima hornu cast kazdeho monitora (pas lisaty + kus plochy) do PNG. Pouzitie: shot-bar.ps1 -Out <prefix> [-Height 80]
# Snimanie cez BitBlt s CAPTUREBLT: bez neho by snimka vynechala vrstvene okna (priehladna lista).
param([string]$Out = "$env:TEMP\transom-shot", [int]$Height = 80)
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class DpiAw {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
  [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
  [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr dst, int x, int y, int w, int h, IntPtr src, int sx, int sy, uint rop);
}
"@
[void][DpiAw]::SetProcessDpiAwarenessContext([IntPtr](-4))   # PerMonitorV2: suradnice vo fyzickych px
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$i = 0
foreach ($s in [System.Windows.Forms.Screen]::AllScreens) {
    $b = $s.Bounds
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $dst = $g.GetHdc(); $scr = [DpiAw]::GetDC([IntPtr]::Zero)
    # SRCCOPY | CAPTUREBLT
    [void][DpiAw]::BitBlt($dst, 0, 0, $b.Width, $Height, $scr, $b.X, $b.Y, 0x00CC0020 -bor 0x40000000)
    [void][DpiAw]::ReleaseDC([IntPtr]::Zero, $scr); $g.ReleaseHdc($dst)
    $f = "$Out-$i.png"; $bmp.Save($f, [System.Drawing.Imaging.ImageFormat]::Png)
    "$f  monitor=$($s.DeviceName) bounds=$b"
    $g.Dispose(); $bmp.Dispose(); $i++
}
