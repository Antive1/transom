# Polozi pod hornu listu (na primarny monitor) bezramove okno s jasnymi pasmi (biela / cervena / siva), aby bolo
# vidiet cez priehladnu listu. Okno je vysoke len ako lista (28 px = rezervovany pas, neprekryva ine okna), nikdy sa
# neaktivuje (WS_EX_NOACTIVATE) a po -Seconds sekundach sa samo zavrie. Nemeni nastavenia systemu ani tapetu.
# Pouzitie (na pozadi): Start-Process pwsh -ArgumentList '-NoProfile','-File','tools\backdrop.ps1','-Seconds','60'
param([int]$Seconds = 60, [int]$Height = 28)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -ReferencedAssemblies System.Windows.Forms, System.Drawing, System.ComponentModel.Primitives, System.Drawing.Primitives, System.Runtime, System.Windows.Forms.Primitives -TypeDefinition @'
using System; using System.Windows.Forms; using System.Runtime.InteropServices;
public class NoActForm : Form {
  protected override bool ShowWithoutActivation { get { return true; } }
  protected override CreateParams CreateParams { get { var cp = base.CreateParams; cp.ExStyle |= 0x08000000 | 0x80; return cp; } }
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
}
'@
[void][NoActForm]::SetProcessDPIAware()
$prim = [System.Windows.Forms.Screen]::AllScreens | Where-Object Primary
$f = New-Object NoActForm
$f.FormBorderStyle = 'None'; $f.ShowInTaskbar = $false; $f.StartPosition = 'Manual'
$f.Bounds = New-Object System.Drawing.Rectangle $prim.Bounds.X, $prim.Bounds.Y, 1200, $Height
$f.BackColor = [System.Drawing.Color]::White
foreach ($band in @(@(400, 'Red'), @(800, 'Gray'))) {
    $p = New-Object System.Windows.Forms.Panel; $p.Left = $band[0]; $p.Top = 0; $p.Width = 400; $p.Height = $Height
    $p.BackColor = if ($band[1] -eq 'Red') { [System.Drawing.Color]::FromArgb(255, 0, 0) } else { [System.Drawing.Color]::FromArgb(128, 128, 128) }
    $f.Controls.Add($p)
}
$t = New-Object System.Windows.Forms.Timer; $t.Interval = $Seconds * 1000; $t.Add_Tick({ $f.Close() }); $t.Start()
[System.Windows.Forms.Application]::Run($f)
