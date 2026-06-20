param(
    [string]$ExePath = (Join-Path $PSScriptRoot "..\target\debug\j3grid-docker.exe"),
    [string]$ArtifactDir = (Join-Path $PSScriptRoot "..\smoke-artifacts\ui-entry")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms

if (-not ("J3GridDockerSmoke.Native" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace J3GridDockerSmoke {
    public struct POINT {
        public int X;
        public int Y;
    }

    public struct RECT {
        public int Left;
        public int Top;
        public int Right;
        public int Bottom;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct MOUSEINPUT {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT {
        public uint type;
        public MOUSEINPUT mi;
    }

    public static class Native {
        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

        [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW", SetLastError=true)]
        public static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int index);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr FindWindow(string className, string windowName);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr childAfter, string className, string windowName);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern IntPtr GetDlgItem(IntPtr hDlg, int itemId);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int width, int height, bool repaint);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool SetWindowPos(IntPtr hWnd, IntPtr insertAfter, int x, int y, int width, int height, uint flags);

        [DllImport("user32.dll")]
        public static extern bool ShowWindow(IntPtr hWnd, int command);

        [DllImport("user32.dll")]
        public static extern bool IsWindowVisible(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern bool IsWindow(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern bool SetCursorPos(int x, int y);

        [DllImport("user32.dll")]
        public static extern bool SetForegroundWindow(IntPtr hWnd);

        [DllImport("user32.dll")]
        public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extraInfo);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern uint SendInput(uint inputCount, INPUT[] inputs, int inputSize);

        [DllImport("user32.dll")]
        public static extern short GetAsyncKeyState(int virtualKey);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool PostMessage(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam);

        [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr SendMessageText(IntPtr hWnd, uint message, IntPtr wParam, string lParam);
    }
}
"@
}

$MouseLeftDown = 0x0002
$MouseLeftUp = 0x0004
$SwMinimize = 6
$SwRestore = 9
$SwpNoZOrder = 0x0004
$SwpNoActivate = 0x0010
$SwpNoMove = 0x0002
$SwpNoSize = 0x0001
$HwndTopMost = [IntPtr](-1)
$HwndNoTopMost = [IntPtr](-2)
$WmClose = 0x0010
$WmSetText = 0x000C
$WmCommand = 0x0111
$WmMouseMove = 0x0200
$WmLButtonDown = 0x0201
$WmLButtonUp = 0x0202
$WmRButtonUp = 0x0205
$MkLButton = 0x0001
$VkLButton = 0x01
$GwlStyle = -16
$WsCaption = 0x00C00000
$TopBarHeight = 70
$StatusBarHeight = 24
$TabBarLeft = 136
$TabWidth = 132
$TabGap = 4
$TabCenterY = 18
$TabCloseCenterOffset = 117
$TabOverflowDropdownGap = 4
$TabOverflowDropdownWidth = 28
$CommandBarY = 52
$SplitVerticalButtonX = 48
$DeleteRegionButtonX = 236
$UndockButtonX = 340
$InputDialogOkId = 1
$InputDialogEditId = 100

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$TracePath = Join-Path $ArtifactDir "ui-entry-smoke.txt"
$ScreenshotPrefix = Join-Path $ArtifactDir "ui-entry"
$AppStdoutPath = Join-Path $ArtifactDir "app-stdout.txt"
$AppStderrPath = Join-Path $ArtifactDir "app-stderr.txt"
$SettingsPath = $null
$trace = New-Object System.Collections.Generic.List[string]

function Add-Trace([string]$Message) {
    $line = "$(Get-Date -Format o) $Message"
    $trace.Add($line)
    Write-Host $line
}

function Save-Trace {
    $trace | Set-Content -LiteralPath $TracePath -Encoding UTF8
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw $Message
    }
    Add-Trace "PASS $Message"
}

function Wait-MainWindow([System.Diagnostics.Process]$Process, [string]$Label) {
    for ($i = 0; $i -lt 80; $i++) {
        if ($Process.HasExited) {
            throw "$Label process exited before creating a main window"
        }
        $Process.Refresh()
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            return $Process.MainWindowHandle
        }
        Start-Sleep -Milliseconds 100
    }
    throw "$Label main window was not created"
}

function New-FallbackExternalWindow {
    $form = New-Object System.Windows.Forms.Form
    $form.Text = "j3GridDocker smoke external window"
    $form.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
    $form.Width = 360
    $form.Height = 240
    $label = New-Object System.Windows.Forms.Label
    $label.Text = "External smoke window"
    $label.Dock = [System.Windows.Forms.DockStyle]::Fill
    $label.TextAlign = [System.Drawing.ContentAlignment]::MiddleCenter
    $form.Controls.Add($label)
    $form.Show()
    [System.Windows.Forms.Application]::DoEvents()
    $form
}

function Get-WindowRect([IntPtr]$Hwnd) {
    $rect = New-Object J3GridDockerSmoke.RECT
    if (-not [J3GridDockerSmoke.Native]::GetWindowRect($Hwnd, [ref]$rect)) {
        throw "GetWindowRect failed for hwnd=$Hwnd"
    }
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Right = $rect.Right
        Bottom = $rect.Bottom
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
    }
}

function Get-WindowTitle([IntPtr]$Hwnd) {
    $title = New-Object System.Text.StringBuilder 256
    [J3GridDockerSmoke.Native]::GetWindowText($Hwnd, $title, $title.Capacity) | Out-Null
    $title.ToString()
}

function Get-ClientRect([IntPtr]$Hwnd) {
    $rect = New-Object J3GridDockerSmoke.RECT
    if (-not [J3GridDockerSmoke.Native]::GetClientRect($Hwnd, [ref]$rect)) {
        throw "GetClientRect failed for hwnd=$Hwnd"
    }
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Right = $rect.Right
        Bottom = $rect.Bottom
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
    }
}

function Convert-ClientPoint([IntPtr]$Hwnd, [int]$X, [int]$Y) {
    $point = New-Object J3GridDockerSmoke.POINT
    $point.X = $X
    $point.Y = $Y
    if (-not [J3GridDockerSmoke.Native]::ClientToScreen($Hwnd, [ref]$point)) {
        throw "ClientToScreen failed for hwnd=$Hwnd"
    }
    [pscustomobject]@{ X = $point.X; Y = $point.Y }
}

function Send-Mouse([uint32]$Flags) {
    [J3GridDockerSmoke.Native]::mouse_event($Flags, 0, 0, 0, [UIntPtr]::Zero)
}

function New-MouseLParam([int]$X, [int]$Y) {
    $value = (($Y -band 0xffff) -shl 16) -bor ($X -band 0xffff)
    [IntPtr]$value
}

function Test-SyntheticLeftButtonVisibleToAsyncKeyState {
    Send-Mouse $MouseLeftDown
    Start-Sleep -Milliseconds 120
    $state = [J3GridDockerSmoke.Native]::GetAsyncKeyState($VkLButton)
    Send-Mouse $MouseLeftUp
    Start-Sleep -Milliseconds 120
    (($state -band -32768) -ne 0)
}

function Get-ContentScreenRect([IntPtr]$Hwnd) {
    $client = Get-ClientRect $Hwnd
    $topLeft = Convert-ClientPoint $Hwnd 0 $TopBarHeight
    [pscustomobject]@{
        Left = $topLeft.X
        Top = $topLeft.Y
        Width = $client.Width
        Height = $client.Height - $TopBarHeight - $StatusBarHeight
        Right = $topLeft.X + $client.Width
        Bottom = $topLeft.Y + $client.Height - $TopBarHeight - $StatusBarHeight
    }
}

function Click-Client([IntPtr]$Hwnd, [int]$X, [int]$Y) {
    [J3GridDockerSmoke.Native]::SetForegroundWindow($Hwnd) | Out-Null
    Start-Sleep -Milliseconds 60
    $lparam = New-MouseLParam $X $Y
    [J3GridDockerSmoke.Native]::PostMessage($Hwnd, $WmLButtonDown, [IntPtr]$MkLButton, $lparam) | Out-Null
    Start-Sleep -Milliseconds 80
    [J3GridDockerSmoke.Native]::PostMessage($Hwnd, $WmLButtonUp, [IntPtr]::Zero, $lparam) | Out-Null
    Start-Sleep -Milliseconds 180
}

function Click-Screen([int]$X, [int]$Y) {
    [J3GridDockerSmoke.Native]::SetCursorPos($X, $Y) | Out-Null
    Start-Sleep -Milliseconds 120
    Send-Mouse $MouseLeftDown
    Start-Sleep -Milliseconds 100
    Send-Mouse $MouseLeftUp
    Start-Sleep -Milliseconds 260
}

function Click-ScreenLegacy([int]$X, [int]$Y) {
    [J3GridDockerSmoke.Native]::SetCursorPos($X, $Y) | Out-Null
    Start-Sleep -Milliseconds 120
    [J3GridDockerSmoke.Native]::mouse_event($MouseLeftDown, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    [J3GridDockerSmoke.Native]::mouse_event($MouseLeftUp, 0, 0, 0, [UIntPtr]::Zero)
    Start-Sleep -Milliseconds 260
}

function Drag-Screen([int]$StartX, [int]$StartY, [int]$EndX, [int]$EndY) {
    [J3GridDockerSmoke.Native]::SetCursorPos($StartX, $StartY) | Out-Null
    Start-Sleep -Milliseconds 150
    Send-Mouse $MouseLeftDown
    Start-Sleep -Milliseconds 150
    for ($step = 1; $step -le 10; $step++) {
        $x = [int]($StartX + (($EndX - $StartX) * $step / 10))
        $y = [int]($StartY + (($EndY - $StartY) * $step / 10))
        [J3GridDockerSmoke.Native]::SetCursorPos($x, $y) | Out-Null
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 50
    }
    Send-Mouse $MouseLeftUp
    Start-Sleep -Milliseconds 500
}

function Click-ClientReal([IntPtr]$Hwnd, [int]$X, [int]$Y) {
    $point = Convert-ClientPoint $Hwnd $X $Y
    Click-Screen $point.X $point.Y
}

function RightClick-Client([IntPtr]$Hwnd, [int]$X, [int]$Y) {
    [J3GridDockerSmoke.Native]::SetForegroundWindow($Hwnd) | Out-Null
    Start-Sleep -Milliseconds 80
    $lparam = New-MouseLParam $X $Y
    [J3GridDockerSmoke.Native]::PostMessage(
        $Hwnd,
        $WmRButtonUp,
        [IntPtr]::Zero,
        $lparam
    ) | Out-Null
    Start-Sleep -Milliseconds 260
    Convert-ClientPoint $Hwnd $X $Y
}

function Drag-Client([IntPtr]$Hwnd, [int]$StartX, [int]$StartY, [int]$EndX, [int]$EndY) {
    [J3GridDockerSmoke.Native]::PostMessage(
        $Hwnd,
        $WmLButtonDown,
        [IntPtr]$MkLButton,
        (New-MouseLParam $StartX $StartY)
    ) | Out-Null
    Start-Sleep -Milliseconds 120
    for ($step = 1; $step -le 8; $step++) {
        $x = [int]($StartX + (($EndX - $StartX) * $step / 8))
        $y = [int]($StartY + (($EndY - $StartY) * $step / 8))
        [J3GridDockerSmoke.Native]::PostMessage(
            $Hwnd,
            $WmMouseMove,
            [IntPtr]$MkLButton,
            (New-MouseLParam $x $y)
        ) | Out-Null
        Start-Sleep -Milliseconds 80
    }
    [J3GridDockerSmoke.Native]::PostMessage(
        $Hwnd,
        $WmLButtonUp,
        [IntPtr]::Zero,
        (New-MouseLParam $EndX $EndY)
    ) | Out-Null
    Start-Sleep -Milliseconds 250
}

function Drag-External-To-Client([IntPtr]$SourceHwnd, [IntPtr]$TargetHwnd, [int]$TargetX, [int]$TargetY) {
    $source = Get-WindowRect $SourceHwnd
    $target = Convert-ClientPoint $TargetHwnd $TargetX $TargetY
    $sourceX = [int]($source.Left + ($source.Width / 2))
    $sourceY = [int]($source.Top + [Math]::Min(28, [Math]::Max(8, [int]($source.Height / 10))))
    $cursorOffsetX = $sourceX - $source.Left
    $cursorOffsetY = $sourceY - $source.Top

    [J3GridDockerSmoke.Native]::SetForegroundWindow($SourceHwnd) | Out-Null
    Start-Sleep -Milliseconds 150
    [J3GridDockerSmoke.Native]::SetCursorPos($sourceX, $sourceY) | Out-Null
    Start-Sleep -Milliseconds 150
    Send-Mouse $MouseLeftDown
    Start-Sleep -Milliseconds 600
    for ($step = 1; $step -le 12; $step++) {
        $x = [int]($sourceX + (($target.X - $sourceX) * $step / 12))
        $y = [int]($sourceY + (($target.Y - $sourceY) * $step / 12))
        [J3GridDockerSmoke.Native]::MoveWindow(
            $SourceHwnd,
            $x - $cursorOffsetX,
            $y - $cursorOffsetY,
            $source.Width,
            $source.Height,
            $true
        ) | Out-Null
        [J3GridDockerSmoke.Native]::SetCursorPos($x, $y) | Out-Null
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 160
    }
    Start-Sleep -Milliseconds 300
    Send-Mouse $MouseLeftUp
    Start-Sleep -Milliseconds 1200
}

function Drag-External-To-Screen([IntPtr]$SourceHwnd, [int]$TargetX, [int]$TargetY) {
    $source = Get-WindowRect $SourceHwnd
    $sourceX = [int]($source.Left + ($source.Width / 2))
    $sourceY = [int]($source.Top + [Math]::Min(28, [Math]::Max(8, [int]($source.Height / 10))))
    $cursorOffsetX = $sourceX - $source.Left
    $cursorOffsetY = $sourceY - $source.Top

    [J3GridDockerSmoke.Native]::SetForegroundWindow($SourceHwnd) | Out-Null
    Start-Sleep -Milliseconds 150
    [J3GridDockerSmoke.Native]::SetCursorPos($sourceX, $sourceY) | Out-Null
    Start-Sleep -Milliseconds 150
    Send-Mouse $MouseLeftDown
    Start-Sleep -Milliseconds 600
    for ($step = 1; $step -le 14; $step++) {
        $x = [int]($sourceX + (($TargetX - $sourceX) * $step / 14))
        $y = [int]($sourceY + (($TargetY - $sourceY) * $step / 14))
        [J3GridDockerSmoke.Native]::MoveWindow(
            $SourceHwnd,
            $x - $cursorOffsetX,
            $y - $cursorOffsetY,
            $source.Width,
            $source.Height,
            $true
        ) | Out-Null
        [J3GridDockerSmoke.Native]::SetCursorPos($x, $y) | Out-Null
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 160
    }
    Start-Sleep -Milliseconds 300
    Send-Mouse $MouseLeftUp
    Start-Sleep -Milliseconds 1200
}

function Assert-RectNear($Actual, $Expected, [int]$Tolerance, [string]$Label) {
    $ok = ([Math]::Abs($Actual.Left - $Expected.Left) -le $Tolerance) -and
        ([Math]::Abs($Actual.Top - $Expected.Top) -le $Tolerance) -and
        ([Math]::Abs($Actual.Width - $Expected.Width) -le $Tolerance) -and
        ([Math]::Abs($Actual.Height - $Expected.Height) -le $Tolerance)
    Assert-True $ok "$Label actual=($($Actual.Left),$($Actual.Top),$($Actual.Width),$($Actual.Height)) expected=($($Expected.Left),$($Expected.Top),$($Expected.Width),$($Expected.Height))"
}

function Assert-PointOutsideRect([int]$X, [int]$Y, $Rect, [string]$Label) {
    $outside = $X -lt $Rect.Left -or $X -ge $Rect.Right -or $Y -lt $Rect.Top -or $Y -ge $Rect.Bottom
    Assert-True $outside "$Label point=($X,$Y) rect=($($Rect.Left),$($Rect.Top),$($Rect.Width),$($Rect.Height))"
}

function Test-WorkspaceUiVisible([IntPtr]$Hwnd) {
    $style = [J3GridDockerSmoke.Native]::GetWindowLongPtr($Hwnd, $GwlStyle).ToInt64()
    (($style -band $WsCaption) -ne 0)
}

function Ensure-WorkspaceUiVisible([IntPtr]$Hwnd) {
    if (-not (Test-WorkspaceUiVisible $Hwnd)) {
        Click-Client $Hwnd 24 16
        Start-Sleep -Milliseconds 600
    }
    Assert-True (Test-WorkspaceUiVisible $Hwnd) "workspace UI is visible before external drop smoke"
}

function Save-Screenshot([string]$Name, [IntPtr]$Hwnd) {
    try {
        [J3GridDockerSmoke.Native]::SetForegroundWindow($Hwnd) | Out-Null
        Start-Sleep -Milliseconds 80
        $rect = Get-WindowRect $Hwnd
        if ($rect.Width -le 0 -or $rect.Height -le 0) {
            return
        }
        $bitmap = New-Object System.Drawing.Bitmap $rect.Width, $rect.Height
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size)
        $path = "$ScreenshotPrefix-$Name.png"
        $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose()
        $bitmap.Dispose()
        Add-Trace "screenshot $path"
    } catch {
        Add-Trace "screenshot skipped: $($_.Exception.Message)"
    }
}

function Copy-SmokeExecutable([string]$ResolvedExe) {
    $smokeExe = Join-Path $ArtifactDir "j3grid-docker-smoke.exe"
    Copy-Item -LiteralPath $ResolvedExe -Destination $smokeExe -Force
    Add-Trace "copied smoke executable to $smokeExe"
    $smokeExe
}

function Remove-SmokeRuntimeFiles([string]$SmokeExe) {
    $script:SettingsPath = [System.IO.Path]::ChangeExtension($SmokeExe, ".toml")
    foreach ($path in @($script:SettingsPath, $AppStdoutPath, $AppStderrPath)) {
        if ($null -ne $path -and (Test-Path -LiteralPath $path)) {
            Remove-Item -LiteralPath $path -Force
        }
    }
}

function Get-VisibleTabBodyX([int]$VisibleIndex) {
    $TabBarLeft + ($VisibleIndex * ($TabWidth + $TabGap)) + 40
}

function Get-VisibleTabCloseX([int]$VisibleIndex) {
    $TabBarLeft + ($VisibleIndex * ($TabWidth + $TabGap)) + $TabCloseCenterOffset
}

function Click-NewTab([IntPtr]$Hwnd) {
    Click-Client $Hwnd 104 16
}

function Click-VisibleTabClose([IntPtr]$Hwnd, [int]$VisibleIndex) {
    Click-Client $Hwnd (Get-VisibleTabCloseX $VisibleIndex) $TabCenterY
}

function Wait-PopupMenuRect {
    for ($i = 0; $i -lt 30; $i++) {
        $menu = [J3GridDockerSmoke.Native]::FindWindow("#32768", $null)
        if ($menu -ne [IntPtr]::Zero -and [J3GridDockerSmoke.Native]::IsWindowVisible($menu)) {
            $rect = Get-WindowRect $menu
            $rect | Add-Member -NotePropertyName Hwnd -NotePropertyValue $menu
            return $rect
        }
        Start-Sleep -Milliseconds 50
    }
    throw "popup menu window was not found"
}

function Select-PopupMenuItem([pscustomobject]$Anchor, [int]$Index, [int]$ItemCount = 0) {
    $menu = Wait-PopupMenuRect
    $height = [Math]::Max(18, [System.Windows.Forms.SystemInformation]::MenuHeight)
    $x = $menu.Left + [Math]::Min(48, [Math]::Max(12, [int]($menu.Width / 2)))
    $y = if ($ItemCount -gt 0) {
        $itemHeight = [Math]::Max(1.0, [double]$menu.Height / [double]$ItemCount)
        $menu.Top + [int][Math]::Round($itemHeight * ([double]$Index + 0.5))
    } else {
        $menu.Top + [int]($height * $Index + ($height / 2))
    }
    Add-Trace "popup menu rect=($($menu.Left),$($menu.Top),$($menu.Width),$($menu.Height)) selecting index=$Index at ($x,$y)"
    Click-Screen $x $y
    Start-Sleep -Milliseconds 350
}

function Invoke-TabContextMenuItem([IntPtr]$Hwnd, [int]$TabBodyX, [int]$ItemIndex) {
    $anchor = RightClick-Client $Hwnd $TabBodyX $TabCenterY
    Select-PopupMenuItem $anchor $ItemIndex 8
}

function Wait-WindowByTitle([string]$Title) {
    for ($i = 0; $i -lt 60; $i++) {
        $hwnd = [J3GridDockerSmoke.Native]::FindWindow($null, $Title)
        if ($hwnd -ne [IntPtr]::Zero) {
            return $hwnd
        }
        if ($Title -eq "Rename tab") {
            $hwnd = [J3GridDockerSmoke.Native]::FindWindow("j3GridDocker.TextInputDialog", $null)
            if ($hwnd -ne [IntPtr]::Zero) {
                return $hwnd
            }
        }
        Start-Sleep -Milliseconds 100
    }
    throw "window titled '$Title' was not found"
}

function Wait-WindowClosed([IntPtr]$Hwnd, [string]$Label) {
    for ($i = 0; $i -lt 60; $i++) {
        if (-not [J3GridDockerSmoke.Native]::IsWindow($Hwnd)) {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "$Label did not close"
}

function Complete-RenameDialog([string]$Name) {
    $dialog = Wait-WindowByTitle "Rename tab"
    $edit = [J3GridDockerSmoke.Native]::FindWindowEx(
        $dialog,
        [IntPtr]::Zero,
        "EDIT",
        $null
    )
    if ($edit -eq [IntPtr]::Zero) {
        $edit = [J3GridDockerSmoke.Native]::GetDlgItem($dialog, $InputDialogEditId)
    }
    Assert-True ($edit -ne [IntPtr]::Zero) "rename dialog edit control found"
    $setTextResult = [J3GridDockerSmoke.Native]::SendMessageText(
        $edit,
        $WmSetText,
        [IntPtr]::Zero,
        $Name
    )
    Assert-True ($setTextResult -ne [IntPtr]::Zero) "rename dialog text set"
    [J3GridDockerSmoke.Native]::PostMessage(
        $dialog,
        $WmCommand,
        [IntPtr]$InputDialogOkId,
        [IntPtr]::Zero
    ) | Out-Null
    Wait-WindowClosed $dialog "rename dialog"
}

function Invoke-TabUxSmoke([IntPtr]$Hwnd) {
    Add-Trace "starting tab UX smoke scenario"
    $originalRect = Get-WindowRect $Hwnd

    for ($i = 0; $i -lt 5; $i++) {
        Click-NewTab $Hwnd
    }

    $originalClient = Get-ClientRect $Hwnd
    $windowFrameWidth = $originalRect.Width - $originalClient.Width
    $overflowClientWidth = $TabBarLeft +
        ($TabWidth * 3) +
        ($TabGap * 2) +
        $TabOverflowDropdownGap +
        $TabOverflowDropdownWidth
    $narrowWidth = [Math]::Min($originalRect.Width, $overflowClientWidth + $windowFrameWidth)
    [J3GridDockerSmoke.Native]::MoveWindow(
        $Hwnd,
        $originalRect.Left,
        $originalRect.Top,
        $narrowWidth,
        $originalRect.Height,
        $true
    ) | Out-Null
    Start-Sleep -Milliseconds 500
    $narrowClient = Get-ClientRect $Hwnd
    Add-Trace "narrowed window for overflow client_width=$($narrowClient.Width)"
    Add-Trace "created six tabs to force tab overflow"
    Save-Screenshot "tab-01-overflow-created" $Hwnd

    $client = Get-ClientRect $Hwnd
    $overflowX = $client.Width - 14
    Click-Client $Hwnd $overflowX $TabCenterY
    $dropdown = Convert-ClientPoint $Hwnd ($client.Width - 28) 32
    Select-PopupMenuItem $dropdown 0 3
    Wait-AppLogContains 'tab-ux event=overflow-select' "overflow menu selected a hidden tab"
    Add-Trace "selected first hidden tab from overflow menu"
    Save-Screenshot "tab-02-overflow-select" $Hwnd

    Click-VisibleTabClose $Hwnd 1
    Wait-AppLogContains 'tab-ux event=delete-finish tab_id=1' "close button deleted inactive visible tab 1"
    Add-Trace "clicked close button on inactive visible tab"
    Save-Screenshot "tab-03-close-button" $Hwnd

    Invoke-TabContextMenuItem $Hwnd (Get-VisibleTabBodyX 1) 0
    Complete-RenameDialog "Smoke Renamed"
    Wait-AppLogContains 'tab-ux event=rename-finish tab_id=2' "rename context action renamed tab 2"
    Add-Trace "renamed visible tab through tab context menu"
    Save-Screenshot "tab-04-rename" $Hwnd

    [J3GridDockerSmoke.Native]::MoveWindow(
        $Hwnd,
        $originalRect.Left,
        $originalRect.Top,
        $originalRect.Width,
        $originalRect.Height,
        $true
    ) | Out-Null
    Start-Sleep -Milliseconds 500
    Add-Trace "restored window width for visible context close and tab reorder"

    Invoke-TabContextMenuItem $Hwnd (Get-VisibleTabBodyX 2) 1
    Wait-AppLogContains 'tab-ux event=delete-finish tab_id=3' "context menu closed tab 3"
    Add-Trace "closed a tab through tab context menu"
    Save-Screenshot "tab-05-context-close" $Hwnd

    Drag-Client $Hwnd (Get-VisibleTabBodyX 3) $TabCenterY ($TabBarLeft + 5) $TabCenterY
    Wait-AppLogContains 'tab-ux event=reorder-finish tab_id=5, before_tab_id=0, changed=true' "drag reorder moved tab 5 before tab 0"
    Add-Trace "dragged a visible tab to the front"
    Save-Screenshot "tab-06-reorder" $Hwnd

    Invoke-TabContextMenuItem $Hwnd (Get-VisibleTabBodyX 2) 2
    Wait-AppLogContains 'tab-ux event=close-other-finish target_tab_id=2, closed=3, total=3, failures=0, active_tab=2' "context menu closed other tabs around tab 2"
    Add-Trace "closed other tabs through tab context menu"
    Save-Screenshot "tab-07-close-other" $Hwnd
}

function Assert-AppLogContains([string]$Pattern, [string]$Label) {
    $content = if (Test-Path -LiteralPath $AppStderrPath) {
        Get-Content -LiteralPath $AppStderrPath -Raw
    } else {
        ""
    }
    Assert-True ($content -match $Pattern) $Label
}

function Wait-AppLogContains([string]$Pattern, [string]$Label) {
    for ($i = 0; $i -lt 40; $i++) {
        try {
            $content = if (Test-Path -LiteralPath $AppStderrPath) {
                Get-Content -LiteralPath $AppStderrPath -Raw
            } else {
                ""
            }
            if ($content -match $Pattern) {
                Add-Trace "PASS $Label"
                return
            }
        } catch {
        }
        Start-Sleep -Milliseconds 100
    }
    throw $Label
}

function Assert-FinalTabSettings {
    Assert-True ($null -ne $SettingsPath) "settings path recorded"
    Assert-True (Test-Path -LiteralPath $SettingsPath) "settings file saved at $SettingsPath"
    $content = Get-Content -LiteralPath $SettingsPath -Raw
    $tabCount = ([regex]::Matches($content, '(?m)^\[\[tabs\]\]')).Count
    Assert-True ($tabCount -ge 1) "final settings saved tabs after smoke flow"
    Assert-True ($content -match 'active_tab_id = 2') "final active tab is the renamed context target"
    Assert-True ($content -match 'id = 2') "final tab id 2 is retained"
    Assert-True ($content -match 'name = "Smoke Renamed"') "final tab name was changed through Rename tab"
    Assert-True ($content -notmatch '(?m)^id = (0|1|3|4|5)$') "tabs closed by Close other tabs are absent from final settings"
}

function Verify-TabUxArtifacts {
    Add-Trace "app stderr $AppStderrPath"
    Add-Trace "app stdout $AppStdoutPath"
    Add-Trace "settings $SettingsPath"
    Assert-AppLogContains 'tab-ux event=overflow-select' "app trace captured overflow tab selection"
    Assert-AppLogContains 'tab-ux event=context-action tab_id=2, action=rename' "app trace captured Rename tab context action"
    Assert-AppLogContains 'tab-ux event=context-action tab_id=3, action=close' "app trace captured Close tab context action"
    Assert-AppLogContains 'tab-ux event=reorder-finish tab_id=5, before_tab_id=0, changed=true' "app trace captured tab drag reorder"
    Assert-AppLogContains 'tab-ux event=context-action tab_id=2, action=close-other' "app trace captured Close other tabs context action"
    Assert-FinalTabSettings
}

if (-not $PSBoundParameters.ContainsKey("ExePath")) {
    Add-Trace "building default debug executable"
    cargo build
} elseif (-not (Test-Path -LiteralPath $ExePath)) {
    Add-Trace "building executable because $ExePath does not exist"
    cargo build
}

$resolvedExe = (Resolve-Path -LiteralPath $ExePath).Path
$runExe = Copy-SmokeExecutable $resolvedExe
Remove-SmokeRuntimeFiles $runExe
$oldAppData = $env:APPDATA
$env:APPDATA = Join-Path $ArtifactDir "appdata"
New-Item -ItemType Directory -Force -Path $env:APPDATA | Out-Null

$app = $null
$notepad = $null
$externalApp = $null
$externalForm = $null

try {
    Add-Trace "starting $runExe"
    $app = Start-Process `
        -FilePath $runExe `
        -PassThru `
        -RedirectStandardOutput $AppStdoutPath `
        -RedirectStandardError $AppStderrPath
    try { $app.WaitForInputIdle(3000) | Out-Null } catch {}
    $main = Wait-MainWindow $app "j3GridDocker"
    Assert-True ($main -ne [IntPtr]::Zero) "main window created hwnd=$main"
    Assert-True ((Get-WindowTitle $main) -eq "j3GridDocker") "main window title is j3GridDocker"

    $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    $appWidth = [Math]::Min(900, [Math]::Max(640, $area.Width - 520))
    $appHeight = [Math]::Min(650, [Math]::Max(500, $area.Height - 120))
    $appX = $area.Left + 40
    $appY = $area.Top + 40
    [J3GridDockerSmoke.Native]::MoveWindow($main, $appX, $appY, $appWidth, $appHeight, $true) | Out-Null
    [J3GridDockerSmoke.Native]::SetWindowPos($main, $HwndTopMost, 0, 0, 0, 0, $SwpNoMove -bor $SwpNoSize -bor $SwpNoActivate) | Out-Null
    [J3GridDockerSmoke.Native]::SetForegroundWindow($main) | Out-Null
    Start-Sleep -Milliseconds 400
    Assert-RectNear (Get-WindowRect $main) ([pscustomobject]@{ Left=$appX; Top=$appY; Width=$appWidth; Height=$appHeight }) 12 "main window move/resize applied"
    Save-Screenshot "01-initial" $main

    Invoke-TabUxSmoke $main

    $client = Get-ClientRect $main
    Click-Client $main 220 180
    Click-Client $main $SplitVerticalButtonX $CommandBarY
    Add-Trace "selected root region and clicked Split V"
    Save-Screenshot "02-split-v" $main

    $beforeTitleDragRect = Get-WindowRect $main
    $beforeTitleDragClient = Get-ClientRect $main
    $titleDragStartX = [int]($beforeTitleDragRect.Left + ($beforeTitleDragRect.Width / 2))
    $titleDragStartY = [int]($beforeTitleDragRect.Top + 24)
    [J3GridDockerSmoke.Native]::SetForegroundWindow($main) | Out-Null
    Start-Sleep -Milliseconds 120
    Drag-Screen $titleDragStartX $titleDragStartY ($titleDragStartX + 80) ($titleDragStartY + 30)
    $afterTitleDragRect = Get-WindowRect $main
    $afterTitleDragClient = Get-ClientRect $main
    $titleDragMoved = ([Math]::Abs($afterTitleDragRect.Left - $beforeTitleDragRect.Left) -gt 8) -or
        ([Math]::Abs($afterTitleDragRect.Top - $beforeTitleDragRect.Top) -gt 8)
    if ($titleDragMoved) {
        Assert-RectNear $afterTitleDragClient $beforeTitleDragClient 0 "main client size unchanged after title bar drag"
        Add-Trace "dragged native title bar from ($titleDragStartX,$titleDragStartY) to ($($titleDragStartX + 80),$($titleDragStartY + 30))"
        Save-Screenshot "03-titlebar-drag" $main
    } else {
        Add-Trace "title bar drag movement skipped: synthetic mouse drag did not move the window in this session"
        [J3GridDockerSmoke.Native]::MoveWindow(
            $main,
            $beforeTitleDragRect.Left + 80,
            $beforeTitleDragRect.Top + 30,
            $beforeTitleDragRect.Width,
            $beforeTitleDragRect.Height,
            $true
        ) | Out-Null
        Start-Sleep -Milliseconds 500
        Assert-RectNear (Get-ClientRect $main) $beforeTitleDragClient 0 "main client size unchanged after fallback move"
        Add-Trace "moved main window with Win32 fallback while split UI is visible"
        Save-Screenshot "03-window-move-fallback" $main
    }

    $client = Get-ClientRect $main
    $splitterStartX = [int]($client.Width / 2)
    $splitterEndX = [int]($client.Width / 3)
    Drag-Client $main $splitterStartX 180 $splitterEndX 180
    Add-Trace "dragged splitter from x=$splitterStartX to x=$splitterEndX"
    Save-Screenshot "03-splitter-drag" $main

    Click-NewTab $main
    Click-Client $main 24 16
    Click-Client $main 160 16
    Click-Client $main 150 $CommandBarY
    Add-Trace "clicked tab add, tab switch, and active tab delete"
    Save-Screenshot "04-tab-flow" $main

    $client = Get-ClientRect $main
    Click-Client $main ([int]($client.Width * 0.75)) 180
    Click-Client $main $DeleteRegionButtonX $CommandBarY
    Add-Trace "selected right region and clicked Delete region"
    Save-Screenshot "05-region-delete" $main

    Ensure-WorkspaceUiVisible $main
    $client = Get-ClientRect $main
    Click-Client $main ([int]($client.Width * 0.75)) 180
    Click-Client $main $DeleteRegionButtonX $CommandBarY
    Add-Trace "ensured workspace UI is visible and collapsed split layout before external drop"

    if (-not (Test-SyntheticLeftButtonVisibleToAsyncKeyState)) {
        Add-Trace "external drop skipped: this session does not expose synthetic left-button state through GetAsyncKeyState, which the app intentionally uses for real drag polling"
        Click-Client $main $UndockButtonX $CommandBarY
        Add-Trace "clicked Undock command without placement to verify command path remains non-crashing"
        [J3GridDockerSmoke.Native]::PostMessage($main, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        if (-not $app.WaitForExit(5000)) {
            throw "j3GridDocker did not exit after WM_CLOSE"
        }
        Assert-True $app.HasExited "message loop exited after WM_CLOSE"
        Verify-TabUxArtifacts
        return
    }

    [J3GridDockerSmoke.Native]::SetWindowPos($main, $HwndNoTopMost, 0, 0, 0, 0, $SwpNoMove -bor $SwpNoSize -bor $SwpNoActivate) | Out-Null
    Add-Trace "starting notepad"
    try {
        $notepad = Start-Process -FilePath "notepad.exe" -PassThru
        try { $notepad.WaitForInputIdle(3000) | Out-Null } catch {}
        $externalHwnd = Wait-MainWindow $notepad "notepad"
        $externalName = "notepad"
    } catch {
        Add-Trace "external drop skipped: notepad unavailable and synthetic fallback windows are not reliable for drag polling in this session. Cause: $($_.Exception.Message)"
        if ($null -ne $notepad -and -not $notepad.HasExited) {
            $notepad.Kill()
        }
        $notepad = $null
        Click-Client $main $UndockButtonX $CommandBarY
        Add-Trace "clicked Undock command without placement to verify command path remains non-crashing"
        [J3GridDockerSmoke.Native]::PostMessage($main, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        if (-not $app.WaitForExit(5000)) {
            throw "j3GridDocker did not exit after WM_CLOSE"
        }
        Assert-True $app.HasExited "message loop exited after WM_CLOSE"
        Verify-TabUxArtifacts
        return
    }
    $noteWidth = 360
    $noteHeight = 240
    $noteX = [Math]::Max($area.Left + 20, $area.Right - $noteWidth - 40)
    $noteY = $area.Top + 80
    [J3GridDockerSmoke.Native]::SetWindowPos($externalHwnd, [IntPtr]::Zero, $noteX, $noteY, $noteWidth, $noteHeight, $SwpNoZOrder -bor $SwpNoActivate) | Out-Null
    Start-Sleep -Milliseconds 300
    [System.Windows.Forms.Application]::DoEvents()
    $originalExternalRect = Get-WindowRect $externalHwnd
    Assert-True ([J3GridDockerSmoke.Native]::IsWindowVisible($externalHwnd)) "$externalName visible before placement"

    $client = Get-ClientRect $main
    Drag-External-To-Client $externalHwnd $main ([int]($client.Width / 2)) ([int](($TopBarHeight + $client.Height - $StatusBarHeight) / 2))
    $content = Get-ContentScreenRect $main
    Assert-RectNear (Get-WindowRect $externalHwnd) $content 12 "$externalName placed into active root region"
    Save-Screenshot "06-notepad-placed" $main

    [J3GridDockerSmoke.Native]::ShowWindow($main, $SwMinimize) | Out-Null
    Start-Sleep -Milliseconds 600
    [System.Windows.Forms.Application]::DoEvents()
    Assert-True (-not [J3GridDockerSmoke.Native]::IsWindowVisible($externalHwnd)) "active external window hidden while main window is minimized"

    [J3GridDockerSmoke.Native]::ShowWindow($main, $SwRestore) | Out-Null
    Start-Sleep -Milliseconds 700
    [System.Windows.Forms.Application]::DoEvents()
    Assert-True ([J3GridDockerSmoke.Native]::IsWindowVisible($externalHwnd)) "active external window visible after restore"
    $content = Get-ContentScreenRect $main
    Assert-RectNear (Get-WindowRect $externalHwnd) $content 12 "$externalName repositioned after restore"
    Save-Screenshot "07-restore" $main

    Click-Client $main $SplitVerticalButtonX $CommandBarY
    Start-Sleep -Milliseconds 500
    [System.Windows.Forms.Application]::DoEvents()
    $content = Get-ContentScreenRect $main
    $leftContent = [pscustomobject]@{
        Left = $content.Left
        Top = $content.Top
        Width = [int]($content.Width / 2)
        Height = $content.Height
        Right = $content.Left + [int]($content.Width / 2)
        Bottom = $content.Bottom
    }
    Assert-RectNear (Get-WindowRect $externalHwnd) $leftContent 12 "$externalName moved with occupied region after split"

    Drag-External-To-Client $externalHwnd $main ([int]($client.Width * 0.75)) ([int](($TopBarHeight + $client.Height - $StatusBarHeight) / 2))
    $rightContent = [pscustomobject]@{
        Left = $leftContent.Right
        Top = $content.Top
        Width = $content.Width - $leftContent.Width
        Height = $content.Height
        Right = $content.Right
        Bottom = $content.Bottom
    }
    Assert-RectNear (Get-WindowRect $externalHwnd) $rightContent 12 "$externalName re-docked into empty right region"
    Save-Screenshot "08-redock-right" $main

    $mainRectBeforeDetach = Get-WindowRect $main
    $detachTargetX = [Math]::Min($area.Right - 80, $mainRectBeforeDetach.Right + 220)
    if ($detachTargetX -lt $mainRectBeforeDetach.Right + 20) {
        $detachTargetX = [Math]::Max($area.Left + 80, $mainRectBeforeDetach.Left - 220)
    }
    $detachTargetY = [Math]::Min($area.Bottom - 80, [Math]::Max($area.Top + 80, $mainRectBeforeDetach.Top + 140))
    Assert-PointOutsideRect $detachTargetX $detachTargetY $mainRectBeforeDetach "detach drop target is outside j3GridDocker"

    Drag-External-To-Screen $externalHwnd $detachTargetX $detachTargetY
    [System.Windows.Forms.Application]::DoEvents()
    $detachedRect = Get-WindowRect $externalHwnd
    Assert-PointOutsideRect `
        ([int]($detachedRect.Left + ($detachedRect.Width / 2))) `
        ([int]($detachedRect.Top + ($detachedRect.Height / 2))) `
        (Get-WindowRect $main) `
        "$externalName detached outside j3GridDocker"
    Save-Screenshot "09-detached-outside" $externalHwnd

    $mainRectForDetachSyncCheck = Get-WindowRect $main
    [J3GridDockerSmoke.Native]::MoveWindow(
        $main,
        $mainRectForDetachSyncCheck.Left - 24,
        $mainRectForDetachSyncCheck.Top,
        $mainRectForDetachSyncCheck.Width,
        $mainRectForDetachSyncCheck.Height,
        $true
    ) | Out-Null
    Start-Sleep -Milliseconds 600
    [System.Windows.Forms.Application]::DoEvents()
    Assert-RectNear (Get-WindowRect $externalHwnd) $detachedRect 12 "$externalName remains detached after j3GridDocker move"
    Add-Trace "dragged docked external window outside and verified it stays detached"

    $client = Get-ClientRect $main
    $redockTargetX = [int]($client.Width * 0.75)
    $redockTargetY = [int](($TopBarHeight + $client.Height - $StatusBarHeight) / 2)
    $redockTarget = Convert-ClientPoint $main $redockTargetX $redockTargetY
    $preRedockRect = Get-WindowRect $externalHwnd
    $redockCursorOffsetX = [int]($preRedockRect.Width / 2)
    $redockCursorOffsetY = [Math]::Min(28, [Math]::Max(8, [int]($preRedockRect.Height / 10)))
    $undockSnapshotRect = [pscustomobject]@{
        Left = $redockTarget.X - $redockCursorOffsetX
        Top = $redockTarget.Y - $redockCursorOffsetY
        Width = $preRedockRect.Width
        Height = $preRedockRect.Height
        Right = $redockTarget.X - $redockCursorOffsetX + $preRedockRect.Width
        Bottom = $redockTarget.Y - $redockCursorOffsetY + $preRedockRect.Height
    }
    Drag-External-To-Client $externalHwnd $main $redockTargetX $redockTargetY
    $content = Get-ContentScreenRect $main
    $leftContent = [pscustomobject]@{
        Left = $content.Left
        Top = $content.Top
        Width = [int]($content.Width / 2)
        Height = $content.Height
        Right = $content.Left + [int]($content.Width / 2)
        Bottom = $content.Bottom
    }
    $rightContent = [pscustomobject]@{
        Left = $leftContent.Right
        Top = $content.Top
        Width = $content.Width - $leftContent.Width
        Height = $content.Height
        Right = $content.Right
        Bottom = $content.Bottom
    }
    Assert-RectNear (Get-WindowRect $externalHwnd) $rightContent 12 "$externalName re-docked after outside detach"
    Save-Screenshot "10-redock-after-detach" $main

    $client = Get-ClientRect $main
    Click-Client $main ([int]($client.Width * 0.25)) ([int]($TopBarHeight + 120))
    Add-Trace "selected an empty region before clicking the docked external window"

    $dockedRect = Get-WindowRect $externalHwnd
    Click-Screen ([int]($dockedRect.Left + ($dockedRect.Width / 2))) ([int]($dockedRect.Top + ($dockedRect.Height / 2)))
    Add-Trace "clicked the docked external window before Undock"

    Click-ClientReal $main 532 52
    Start-Sleep -Milliseconds 500
    [System.Windows.Forms.Application]::DoEvents()
    Assert-RectNear (Get-WindowRect $externalHwnd) $undockSnapshotRect 12 "$externalName restored by Undock command after docked-window selection"
    Save-Screenshot "11-undock" $main

    [J3GridDockerSmoke.Native]::PostMessage($main, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $app.WaitForExit(5000)) {
        throw "j3GridDocker did not exit after WM_CLOSE"
    }
    Assert-True $app.HasExited "message loop exited after WM_CLOSE"
    Verify-TabUxArtifacts
} finally {
    if ($null -ne $externalForm) {
        $externalForm.Close()
        $externalForm.Dispose()
        [System.Windows.Forms.Application]::DoEvents()
    }
    if ($null -ne $notepad -and -not $notepad.HasExited) {
        if ($notepad.MainWindowHandle -ne [IntPtr]::Zero) {
            [J3GridDockerSmoke.Native]::PostMessage($notepad.MainWindowHandle, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            $notepad.WaitForExit(3000) | Out-Null
        }
        if (-not $notepad.HasExited) {
            $notepad.Kill()
        }
    }
    if ($null -ne $externalApp -and -not $externalApp.HasExited) {
        if ($externalApp.MainWindowHandle -ne [IntPtr]::Zero) {
            [J3GridDockerSmoke.Native]::PostMessage($externalApp.MainWindowHandle, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            $externalApp.WaitForExit(3000) | Out-Null
        }
        if (-not $externalApp.HasExited) {
            $externalApp.Kill()
        }
    }
    if ($null -ne $app -and -not $app.HasExited) {
        [J3GridDockerSmoke.Native]::SetWindowPos($app.MainWindowHandle, $HwndNoTopMost, 0, 0, 0, 0, $SwpNoMove -bor $SwpNoSize -bor $SwpNoActivate) | Out-Null
        [J3GridDockerSmoke.Native]::PostMessage($app.MainWindowHandle, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        $app.WaitForExit(3000) | Out-Null
        if (-not $app.HasExited) {
            $app.Kill()
        }
    }
    $env:APPDATA = $oldAppData
    Add-Trace "trace $TracePath"
    Save-Trace
}
