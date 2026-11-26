Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices;
public class SN {
 [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
 [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string c, string t);
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
 [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
 [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
 public static string Bar(string title) { var h = FindWindow(null, title); if (h == IntPtr.Zero) return title + ": nenajdene"; RECT r; GetWindowRect(h, out r); return string.Format("{0}: rect=({1},{2})-({3},{4}) vyska={5} dpi={6}", title, r.L, r.T, r.R, r.B, r.B - r.T, GetDpiForWindow(h)); }
}
'@
[void][SN]::SetProcessDpiAwarenessContext([IntPtr](-4))
"--- $(Get-Date -Format HH:mm:ss)"
[Windows.Forms.Screen]::AllScreens | % { "{0} bounds={1} work={2} primary={3}" -f $_.DeviceName, $_.Bounds, $_.WorkingArea, $_.Primary }
[SN]::Bar("transom-bar-0"); [SN]::Bar("transom-bar-1")
