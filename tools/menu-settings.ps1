# Overi kontextove menu listy: prave tlacidlo na prazdnom mieste, vypise pristupne nazvy poloziek (UI Automation),
# Enter na prvej polozke ("Nastavenia") a vypise popredne okno. Kurzor vrati spat. Nemeni systemove nastavenia.
# Pouzitie: menu-settings.ps1 -X 1500 -Y 14
param([int]$X, [int]$Y, [int]$WaitMs = 1500)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System; using System.Text; using System.Runtime.InteropServices; using System.Diagnostics;
public static class MS {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, IntPtr e);
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte sc, uint f, IntPtr e);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
  public static string Fg() {
    var h = GetForegroundWindow(); var c = new StringBuilder(256); var t = new StringBuilder(256);
    GetClassName(h, c, 256); GetWindowText(h, t, 256);
    return string.Format("class='{0}' title='{1}'", c, t);
  }
}
'@
[void][MS]::SetProcessDpiAwarenessContext([IntPtr](-4))
$p = New-Object MS+POINT; [void][MS]::GetCursorPos([ref]$p)
"pred: $([MS]::Fg())"
[void][MS]::SetCursorPos($X, $Y); Start-Sleep -Milliseconds 150
[MS]::mouse_event(0x0008, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 40
[MS]::mouse_event(0x0010, 0, 0, 0, [IntPtr]::Zero); Start-Sleep -Milliseconds 700
$root = [System.Windows.Automation.AutomationElement]::RootElement
$cond = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::MenuItem)
$items = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $cond)
foreach ($i in $items) { "polozka menu: '$($i.Current.Name)'" }
# Enter = prva polozka (Nastavenia); menu ju ma po otvoreni klavesnicou zvyraznenu az po sipke dole
[MS]::keybd_event(0x28, 0, 0, [IntPtr]::Zero); [MS]::keybd_event(0x28, 0, 2, [IntPtr]::Zero); Start-Sleep -Milliseconds 100
[MS]::keybd_event(0x0D, 0, 0, [IntPtr]::Zero); [MS]::keybd_event(0x0D, 0, 2, [IntPtr]::Zero)
Start-Sleep -Milliseconds $WaitMs
"po:   $([MS]::Fg())"
[void][MS]::SetCursorPos($p.X, $p.Y)
