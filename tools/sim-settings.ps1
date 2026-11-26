# Simulacia systemovych udalosti pre test lisaty: theme = prepne SystemUsesLightTheme a rozposle WM_SETTINGCHANGE
# "ImmersiveColorSet" (ako to robi Windows); timechange = rozposle WM_TIMECHANGE. Pri 'theme' vrati povodnu hodnotu
# az po -HoldSec sekundach. Pouzitie: sim-settings.ps1 -What theme|timechange [-HoldSec 3]
param([ValidateSet("theme", "timechange")][string]$What, [int]$HoldSec = 3)
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices;
public static class Sim {
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, string l, uint f, uint t, out IntPtr r);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r);
}
"@
$HWND_BROADCAST = [IntPtr]0xffff; $r = [IntPtr]::Zero
if ($What -eq "timechange") {
    [void][Sim]::SendMessageTimeout($HWND_BROADCAST, 0x1E, [IntPtr]::Zero, [IntPtr]::Zero, 2, 2000, [ref]$r)
    "WM_TIMECHANGE rozoslane"; return
}
$k = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"
$old = (Get-ItemProperty $k).SystemUsesLightTheme
"povodne SystemUsesLightTheme=$old"
try {
    Set-ItemProperty $k SystemUsesLightTheme ([int](1 - $old)) -Type DWord
    [void][Sim]::SendMessageTimeout($HWND_BROADCAST, 0x1A, [IntPtr]::Zero, "ImmersiveColorSet", 2, 2000, [ref]$r)
    "prepnute na $(1 - $old), broadcast poslany"
    Start-Sleep $HoldSec
} finally {
    Set-ItemProperty $k SystemUsesLightTheme ([int]$old) -Type DWord
    [void][Sim]::SendMessageTimeout($HWND_BROADCAST, 0x1A, [IntPtr]::Zero, "ImmersiveColorSet", 2, 2000, [ref]$r)
    "vratene na $old"
}
