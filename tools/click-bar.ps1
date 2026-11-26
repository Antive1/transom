# Klikne na lištu transom na danom bode (fyzicke px), pocka a vypise popredne okno a prvok s fokusom (UI Automation).
# Potom (volitelne) posle Esc a vrati kurzor na povodne miesto. Nemeni systemove nastavenia.
# Pouzitie: click-bar.ps1 -X 18 -Y 14 [-WaitMs 1200] [-Esc] [-Right] [-Label search]   (-Right = prave tlacidlo)
param([int]$X, [int]$Y, [int]$WaitMs = 1200, [switch]$Esc, [switch]$Right, [string]$Label = "click")
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices; using System.Diagnostics;
public static class CB {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, IntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static string Fg() {
    var h = GetForegroundWindow(); var c = new StringBuilder(256); var t = new StringBuilder(256);
    GetClassName(h, c, 256); GetWindowText(h, t, 256); uint pid; GetWindowThreadProcessId(h, out pid);
    string exe = "?"; try { exe = Process.GetProcessById((int)pid).ProcessName; } catch {}
    return string.Format("hwnd=0x{0:X} class='{1}' title='{2}' exe={3}", h.ToInt64(), c, t, exe);
  }
}
'@
[void][CB]::SetProcessDpiAwarenessContext([IntPtr](-4))
$p = New-Object CB+POINT; [void][CB]::GetCursorPos([ref]$p)
"[$Label] pred:  $([CB]::Fg())"
[void][CB]::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 150
$down, $up = if ($Right) { 0x0008, 0x0010 } else { 0x0002, 0x0004 }
[CB]::mouse_event($down, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
[CB]::mouse_event($up, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds $WaitMs
"[$Label] po:    $([CB]::Fg())"
try {
  $f = [System.Windows.Automation.AutomationElement]::FocusedElement
  "[$Label] fokus: name='$($f.Current.Name)' automationId='$($f.Current.AutomationId)' class='$($f.Current.ClassName)'"
} catch { "[$Label] fokus: nezistitelny ($_)" }
if ($Esc) { [CB]::keybd_event(0x1B, 0, 0, [IntPtr]::Zero); [CB]::keybd_event(0x1B, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 500; "[$Label] po Esc: $([CB]::Fg())" }
[void][CB]::SetCursorPos($p.X, $p.Y)
