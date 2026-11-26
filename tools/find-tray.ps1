# Vyhlada ikony oblasti oznameni (a skryte ikony) podla nazvu cez UI Automation a vypise ich obdlznik (fyzicke px).
# Pouzitie: find-tray.ps1 [-Name transom] [-Click] [-Right]
#   -Click  = lavy klik na ikonu (okno nastaveni), -Right = pravy klik (kontextove menu).
# Nic nemeni v systeme; klik sa vykona len na vyziadanie. Skryte ikony sa hladaju v "Zobrazit skryte ikony".
param([string]$Name = "transom", [switch]$Click, [switch]$Right)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class TrayMouse {
  [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
}
'@
[void][TrayMouse]::SetProcessDpiAwarenessContext([IntPtr](-4))
$root = [System.Windows.Automation.AutomationElement]::RootElement
$all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
  (New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)))
$found = @()
foreach ($e in $all) {
  try { $n = $e.Current.Name } catch { continue }
  if ($n -like "*$Name*") {
    $r = $e.Current.BoundingRectangle
    $found += [pscustomobject]@{ Name = $n; Class = $e.Current.ClassName; X = [int]$r.X; Y = [int]$r.Y; W = [int]$r.Width; H = [int]$r.Height; Offscreen = $e.Current.IsOffscreen }
  }
}
$found | Format-Table -AutoSize | Out-String -Width 200
if (($Click -or $Right) -and $found.Count -gt 0) {
  $f = $found | Where-Object { -not $_.Offscreen } | Select-Object -First 1
  if (-not $f) { "ikona je skryta (Offscreen)"; return }
  $cx = $f.X + [int]($f.W / 2); $cy = $f.Y + [int]($f.H / 2)
  [void][TrayMouse]::SetCursorPos($cx, $cy); Start-Sleep -Milliseconds 150
  if ($Right) { [TrayMouse]::mouse_event(0x08, 0, 0, 0, [UIntPtr]::Zero); [TrayMouse]::mouse_event(0x10, 0, 0, 0, [UIntPtr]::Zero) }
  else { [TrayMouse]::mouse_event(0x02, 0, 0, 0, [UIntPtr]::Zero); [TrayMouse]::mouse_event(0x04, 0, 0, 0, [UIntPtr]::Zero) }
  "klik na ($cx,$cy)"
}
