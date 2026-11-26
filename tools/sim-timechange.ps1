# Posle WM_TIMECHANGE (0x1E) priamo vsetkym oknam procesu transom-app (najde ich cez EnumWindows podla PID),
# lebo FindWindow ani broadcast riadiace okno nenasli. Vypise nazvy tried a titulky oken.
Add-Type -TypeDefinition @"
using System; using System.Runtime.InteropServices; using System.Text; using System.Collections.Generic;
public static class T {
 delegate bool EnumProc(IntPtr h, IntPtr l);
 [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll")] static extern IntPtr SendMessageTimeout(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r);
 public static string Send(uint pid, uint msg) {
   var res = new List<string>();
   EnumWindows((h, l) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid) {
       var c = new StringBuilder(256); var t = new StringBuilder(256); GetClassName(h, c, 256); GetWindowText(h, t, 256);
       IntPtr r; SendMessageTimeout(h, msg, IntPtr.Zero, IntPtr.Zero, 2, 2000, out r); res.Add(c + "/" + t); } return true; }, IntPtr.Zero);
   return string.Join(", ", res);
 }
}
"@
$p = Get-Process transom-app -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { "transom-app nebezi"; return }
"okna procesu $($p.Id): " + [T]::Send([uint32]$p.Id, 0x1E)
