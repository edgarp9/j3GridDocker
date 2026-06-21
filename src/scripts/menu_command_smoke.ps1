param(
    [string]$ExePath = (Join-Path $PSScriptRoot "..\target\debug\j3grid-docker.exe"),
    [string]$ArtifactDir = (Join-Path $PSScriptRoot "..\smoke-artifacts\menu-command")
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Add-Type -AssemblyName System.Windows.Forms

if (-not ("J3GridDockerMenuSmoke.Native" -as [type])) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace J3GridDockerMenuSmoke {
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

    public static class Native {
        public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint processId);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern int GetClassName(IntPtr hWnd, StringBuilder className, int maxCount);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool IsWindowVisible(IntPtr hWnd);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool GetClientRect(IntPtr hWnd, out RECT rect);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool ClientToScreen(IntPtr hWnd, ref POINT point);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern int GetWindowText(IntPtr hWnd, StringBuilder text, int maxCount);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr FindWindow(string className, string windowName);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr childAfter, string className, string windowName);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern IntPtr GetDlgItem(IntPtr hDlg, int itemId);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool IsWindow(IntPtr hWnd);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool IsIconic(IntPtr hWnd);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool IsZoomed(IntPtr hWnd);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int width, int height, bool repaint);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool PostMessage(IntPtr hWnd, uint message, IntPtr wParam, IntPtr lParam);

        [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern IntPtr SendMessageText(IntPtr hWnd, uint message, IntPtr wParam, string lParam);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern bool ShowWindow(IntPtr hWnd, int command);

        [DllImport("user32.dll", SetLastError=true)]
        public static extern IntPtr GetMenu(IntPtr hWnd);

        [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
        public static extern int GetMenuString(IntPtr hMenu, uint uIDItem, StringBuilder lpString, int cchMax, uint flags);
    }
}
"@
}

$CmdTabAdd = 1001
$CmdSplitVertical = 1003
$CmdSplitHorizontal = 1004
$CmdRegionDelete = 1005
$CmdUndock = 1006
$CmdWorkspaceUiToggle = 1007
$CmdAbout = 1008
$CmdTabRenameContext = 1009
$CmdTabCloseContext = 1010
$CmdTabCloseOtherContext = 1011
$CmdDockHiddenWorkspaceUiToggle = 1016
$CmdWindowMinimize = 1018
$CmdWindowMaximizeRestore = 1019
$CmdWindowClose = 1020
$CmdLanguageEnglish = 1022
$CmdLanguageKorean = 1023
$CmdTabPresetSave = 1024
$CmdTabPresetBase = 3000
$CmdTabPresetDeleteBase = 4000
$CmdTabPresetEditBase = 5000

$IdOk = 1
$InputDialogEditId = 100
$MfByPosition = 0x00000400
$SwRestore = 9
$BmClick = 0x00F5
$WmCommand = 0x0111
$WmClose = 0x0010
$WmSetText = 0x000C
$WmLButtonDown = 0x0201
$WmLButtonUp = 0x0202
$MkLButton = 0x0001

New-Item -ItemType Directory -Force -Path $ArtifactDir | Out-Null
$TracePath = Join-Path $ArtifactDir "menu-command-smoke.txt"
$AppStdoutPath = Join-Path $ArtifactDir "app-stdout.txt"
$AppStderrPath = Join-Path $ArtifactDir "app-stderr.txt"
$SettingsPath = $null
$AppProcessId = 0
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

function Get-WindowTitle([IntPtr]$Hwnd) {
    $title = New-Object System.Text.StringBuilder 256
    [J3GridDockerMenuSmoke.Native]::GetWindowText($Hwnd, $title, $title.Capacity) | Out-Null
    $title.ToString()
}

function Get-WindowRect([IntPtr]$Hwnd) {
    $rect = New-Object J3GridDockerMenuSmoke.RECT
    if (-not [J3GridDockerMenuSmoke.Native]::GetWindowRect($Hwnd, [ref]$rect)) {
        throw "GetWindowRect failed for hwnd=$Hwnd"
    }
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
    }
}

function Get-ClientRect([IntPtr]$Hwnd) {
    $rect = New-Object J3GridDockerMenuSmoke.RECT
    if (-not [J3GridDockerMenuSmoke.Native]::GetClientRect($Hwnd, [ref]$rect)) {
        throw "GetClientRect failed for hwnd=$Hwnd"
    }
    [pscustomobject]@{
        Left = $rect.Left
        Top = $rect.Top
        Width = $rect.Right - $rect.Left
        Height = $rect.Bottom - $rect.Top
    }
}

function New-MouseLParam([int]$X, [int]$Y) {
    $value = (($Y -band 0xffff) -shl 16) -bor ($X -band 0xffff)
    [IntPtr]$value
}

function Click-Client([IntPtr]$Hwnd, [int]$X, [int]$Y) {
    $lparam = New-MouseLParam $X $Y
    [J3GridDockerMenuSmoke.Native]::PostMessage($Hwnd, $WmLButtonDown, [IntPtr]$MkLButton, $lparam) | Out-Null
    Start-Sleep -Milliseconds 80
    [J3GridDockerMenuSmoke.Native]::PostMessage($Hwnd, $WmLButtonUp, [IntPtr]::Zero, $lparam) | Out-Null
    Start-Sleep -Milliseconds 240
}

function Invoke-CommandId([IntPtr]$Hwnd, [int]$Command, [int]$DelayMs = 300) {
    Assert-True ([J3GridDockerMenuSmoke.Native]::PostMessage($Hwnd, $WmCommand, [IntPtr]$Command, [IntPtr]::Zero)) "posted command $Command"
    Start-Sleep -Milliseconds $DelayMs
    [System.Windows.Forms.Application]::DoEvents()
}

function Wait-Window([string]$Title, [string]$ClassName) {
    for ($i = 0; $i -lt 80; $i++) {
        $hwnd = Find-ProcessWindow $script:AppProcessId $Title $ClassName
        if ($Title.Length -gt 0) {
            $candidate = [J3GridDockerMenuSmoke.Native]::FindWindow($null, $Title)
            if ($hwnd -eq [IntPtr]::Zero -and (Test-WindowProcess $candidate $script:AppProcessId)) {
                $hwnd = $candidate
            }
        }
        if ($hwnd -eq [IntPtr]::Zero -and $ClassName.Length -gt 0) {
            $candidate = [J3GridDockerMenuSmoke.Native]::FindWindow($ClassName, $null)
            if (Test-WindowProcess $candidate $script:AppProcessId) {
                $hwnd = $candidate
            }
        }
        if ($hwnd -ne [IntPtr]::Zero) {
            return $hwnd
        }
        Start-Sleep -Milliseconds 100
    }
    Add-Trace "window search failed; process windows: $(Get-ProcessWindowSummary $script:AppProcessId)"
    throw "window title='$Title' class='$ClassName' was not found"
}

function Test-WindowProcess([IntPtr]$Hwnd, [int]$ProcessId) {
    if ($Hwnd -eq [IntPtr]::Zero -or $ProcessId -le 0) {
        return $false
    }
    $windowPid = [uint32]0
    [J3GridDockerMenuSmoke.Native]::GetWindowThreadProcessId($Hwnd, [ref]$windowPid) | Out-Null
    [int]$windowPid -eq $ProcessId
}

function Find-ProcessWindow([int]$ProcessId, [string]$Title, [string]$ClassName) {
    if ($ProcessId -le 0) {
        return [IntPtr]::Zero
    }

    $result = [IntPtr]::Zero
    $callback = [J3GridDockerMenuSmoke.Native+EnumWindowsProc]{
        param([IntPtr]$Hwnd, [IntPtr]$LParam)

        $windowPid = [uint32]0
        [J3GridDockerMenuSmoke.Native]::GetWindowThreadProcessId($Hwnd, [ref]$windowPid) | Out-Null
        if ([int]$windowPid -ne $ProcessId) {
            return $true
        }

        $titleBuilder = New-Object System.Text.StringBuilder 256
        [J3GridDockerMenuSmoke.Native]::GetWindowText($Hwnd, $titleBuilder, $titleBuilder.Capacity) | Out-Null
        $classBuilder = New-Object System.Text.StringBuilder 256
        [J3GridDockerMenuSmoke.Native]::GetClassName($Hwnd, $classBuilder, $classBuilder.Capacity) | Out-Null

        $titleMatches = $Title.Length -eq 0 -or $titleBuilder.ToString() -eq $Title
        $classMatches = $ClassName.Length -eq 0 -or $classBuilder.ToString() -eq $ClassName
        if ($titleMatches -and $classMatches) {
            $script:FoundWindow = $Hwnd
            return $false
        }
        return $true
    }

    $script:FoundWindow = [IntPtr]::Zero
    [J3GridDockerMenuSmoke.Native]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
    $result = $script:FoundWindow
    $script:FoundWindow = [IntPtr]::Zero
    $result
}

function Get-ProcessWindowSummary([int]$ProcessId) {
    if ($ProcessId -le 0) {
        return "process id is not available"
    }

    $items = New-Object System.Collections.Generic.List[string]
    $callback = [J3GridDockerMenuSmoke.Native+EnumWindowsProc]{
        param([IntPtr]$Hwnd, [IntPtr]$LParam)

        $windowPid = [uint32]0
        [J3GridDockerMenuSmoke.Native]::GetWindowThreadProcessId($Hwnd, [ref]$windowPid) | Out-Null
        if ([int]$windowPid -eq $ProcessId) {
            $title = New-Object System.Text.StringBuilder 256
            [J3GridDockerMenuSmoke.Native]::GetWindowText($Hwnd, $title, $title.Capacity) | Out-Null
            $className = New-Object System.Text.StringBuilder 256
            [J3GridDockerMenuSmoke.Native]::GetClassName($Hwnd, $className, $className.Capacity) | Out-Null
            $visible = [J3GridDockerMenuSmoke.Native]::IsWindowVisible($Hwnd)
            $items.Add("hwnd=$Hwnd class='$($className.ToString())' title='$($title.ToString())' visible=$visible")
        }
        return $true
    }
    [J3GridDockerMenuSmoke.Native]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null
    if ($items.Count -eq 0) {
        return "none"
    }
    $items -join "; "
}

function Wait-WindowClosed([IntPtr]$Hwnd, [string]$Label) {
    for ($i = 0; $i -lt 80; $i++) {
        if (-not [J3GridDockerMenuSmoke.Native]::IsWindow($Hwnd)) {
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "$Label did not close"
}

function Complete-RenameDialog([string]$Name) {
    $dialog = Wait-Window "Rename tab" "j3GridDocker.TextInputDialog"
    $edit = [J3GridDockerMenuSmoke.Native]::FindWindowEx($dialog, [IntPtr]::Zero, "EDIT", $null)
    if ($edit -eq [IntPtr]::Zero) {
        $edit = [J3GridDockerMenuSmoke.Native]::GetDlgItem($dialog, $InputDialogEditId)
    }
    Assert-True ($edit -ne [IntPtr]::Zero) "rename dialog edit control found"
    [J3GridDockerMenuSmoke.Native]::SendMessageText($edit, $WmSetText, [IntPtr]::Zero, $Name) | Out-Null
    [J3GridDockerMenuSmoke.Native]::PostMessage($dialog, $WmCommand, [IntPtr]$IdOk, [IntPtr]::Zero) | Out-Null
    Wait-WindowClosed $dialog "rename dialog"
    Add-Trace "completed Rename tab dialog"
}

function Complete-ProgramEditDialog {
    $dialog = Wait-Window "Edit tab preset" "j3GridDocker.ProgramEditDialog"
    [J3GridDockerMenuSmoke.Native]::PostMessage($dialog, $WmCommand, [IntPtr]$IdOk, [IntPtr]::Zero) | Out-Null
    Wait-WindowClosed $dialog "program edit dialog"
    Add-Trace "completed tab preset edit dialog"
    Start-Sleep -Milliseconds 250
}

function Complete-AboutDialog {
    $dialog = Wait-Window "" "j3GridDocker.AboutDialog"
    Add-Trace "about dialog hwnd=$dialog windows=$(Get-ProcessWindowSummary $script:AppProcessId)"
    $ok = [J3GridDockerMenuSmoke.Native]::GetDlgItem($dialog, $IdOk)
    if ($ok -ne [IntPtr]::Zero) {
        [J3GridDockerMenuSmoke.Native]::PostMessage($ok, $BmClick, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 300
    }
    if ([J3GridDockerMenuSmoke.Native]::IsWindow($dialog)) {
        [J3GridDockerMenuSmoke.Native]::PostMessage($dialog, $WmCommand, [IntPtr]$IdOk, [IntPtr]::Zero) | Out-Null
        Start-Sleep -Milliseconds 300
    }
    if ([J3GridDockerMenuSmoke.Native]::IsWindow($dialog)) {
        [J3GridDockerMenuSmoke.Native]::PostMessage($dialog, $WmClose, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    }
    Wait-WindowClosed $dialog "about dialog"
    Add-Trace "completed About dialog"
}

function Get-TopMenuText([IntPtr]$Hwnd, [int]$Index) {
    $menu = [J3GridDockerMenuSmoke.Native]::GetMenu($Hwnd)
    if ($menu -eq [IntPtr]::Zero) {
        return $null
    }
    $text = New-Object System.Text.StringBuilder 128
    [J3GridDockerMenuSmoke.Native]::GetMenuString($menu, [uint32]$Index, $text, $text.Capacity, $MfByPosition) | Out-Null
    $text.ToString()
}

function Assert-MenuVisible([IntPtr]$Hwnd, [string]$Label) {
    $menu = [J3GridDockerMenuSmoke.Native]::GetMenu($Hwnd)
    Assert-True ($menu -ne [IntPtr]::Zero) $Label
}

function Assert-MenuHidden([IntPtr]$Hwnd, [string]$Label) {
    $menu = [J3GridDockerMenuSmoke.Native]::GetMenu($Hwnd)
    Assert-True ($menu -eq [IntPtr]::Zero) $Label
}

function Copy-SmokeExecutable([string]$ResolvedExe) {
    $smokeExe = Join-Path $ArtifactDir "j3grid-docker-menu-smoke.exe"
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

$app = $null
try {
    Add-Trace "starting $runExe"
    $app = Start-Process `
        -FilePath $runExe `
        -PassThru `
        -RedirectStandardOutput $AppStdoutPath `
        -RedirectStandardError $AppStderrPath
    $script:AppProcessId = $app.Id
    try { $app.WaitForInputIdle(3000) | Out-Null } catch {}
    $main = Wait-MainWindow $app "j3GridDocker"
    Assert-True ((Get-WindowTitle $main) -eq "j3GridDocker") "main window title is j3GridDocker"

    $area = [System.Windows.Forms.Screen]::PrimaryScreen.WorkingArea
    [J3GridDockerMenuSmoke.Native]::MoveWindow($main, $area.Left + 60, $area.Top + 60, 920, 640, $true) | Out-Null
    Start-Sleep -Milliseconds 300

    Assert-MenuVisible $main "native main menu is visible initially"
    Assert-True ((Get-TopMenuText $main 0) -eq "Workspace") "English Workspace menu label is visible"

    Invoke-CommandId $main $CmdTabAdd
    Invoke-CommandId $main $CmdTabRenameContext
    Complete-RenameDialog "Menu Smoke"
    Invoke-CommandId $main $CmdTabCloseOtherContext
    Invoke-CommandId $main $CmdTabAdd
    Invoke-CommandId $main $CmdTabCloseContext
    Add-Trace "Workspace menu commands executed"

    $client = Get-ClientRect $main
    Click-Client $main ([int]($client.Width / 2)) 180
    Invoke-CommandId $main $CmdSplitVertical
    Click-Client $main ([int]($client.Width * 0.75)) 180
    Invoke-CommandId $main $CmdSplitHorizontal
    Click-Client $main ([int]($client.Width * 0.75)) 260
    Invoke-CommandId $main $CmdRegionDelete
    Invoke-CommandId $main $CmdUndock
    Add-Trace "Layout menu commands executed"

    Invoke-CommandId $main $CmdTabPresetSave 500
    Complete-ProgramEditDialog
    Invoke-CommandId $main $CmdTabPresetBase
    Invoke-CommandId $main $CmdTabPresetEditBase 500
    Complete-ProgramEditDialog
    Invoke-CommandId $main $CmdTabPresetDeleteBase
    Invoke-CommandId $main $CmdTabPresetSave 500
    Complete-ProgramEditDialog
    Add-Trace "Presets menu commands executed"

    Invoke-CommandId $main $CmdWorkspaceUiToggle
    Assert-MenuHidden $main "native main menu is hidden with workspace controls"
    Invoke-CommandId $main $CmdWorkspaceUiToggle
    Assert-MenuVisible $main "native main menu is restored with workspace controls"

    Invoke-CommandId $main $CmdDockHiddenWorkspaceUiToggle
    Invoke-CommandId $main $CmdLanguageKorean
    Assert-True ((Get-TopMenuText $main 0) -ne "Workspace") "language command changed top-level menu text"
    Invoke-CommandId $main $CmdLanguageEnglish
    Assert-True ((Get-TopMenuText $main 0) -eq "Workspace") "language command restored English top-level menu text"
    Invoke-CommandId $main $CmdLanguageKorean
    Add-Trace "View and Options menu commands executed"

    Invoke-CommandId $main $CmdWindowMinimize
    Assert-True ([J3GridDockerMenuSmoke.Native]::IsIconic($main)) "Window menu minimized main window"
    [J3GridDockerMenuSmoke.Native]::ShowWindow($main, $SwRestore) | Out-Null
    Start-Sleep -Milliseconds 400
    Invoke-CommandId $main $CmdWindowMaximizeRestore
    Assert-True ([J3GridDockerMenuSmoke.Native]::IsZoomed($main)) "Window menu maximized main window"
    Invoke-CommandId $main $CmdWindowMaximizeRestore
    Start-Sleep -Milliseconds 400
    Assert-True (-not [J3GridDockerMenuSmoke.Native]::IsZoomed($main)) "Window menu restored main window"

    Invoke-CommandId $main $CmdAbout 200
    Complete-AboutDialog
    Add-Trace "Window and Help menu commands executed"

    Invoke-CommandId $main $CmdWindowClose
    if (-not $app.WaitForExit(5000)) {
        throw "j3GridDocker did not exit after Window > Close"
    }
    Assert-True $app.HasExited "Window Close command exited the message loop"

    Assert-True (Test-Path -LiteralPath $SettingsPath) "settings file was saved at $SettingsPath"
    $content = Get-Content -LiteralPath $SettingsPath -Raw
    Assert-True ($content -match 'dock_hidden_workspace_ui = true') "Dock hidden workspace option persisted"
    Assert-True ($content -match 'ui_language = "korean"') "Korean UI language option persisted"
    Assert-True ($content -match '\[\[tab_presets\]\]') "tab preset persisted after menu flow"
    Assert-True ($content -match 'name = "Tab Preset 1"') "tab preset load renamed the active tab to the preset name"
    $appLog = if (Test-Path -LiteralPath $AppStderrPath) {
        Get-Content -LiteralPath $AppStderrPath -Raw
    } else {
        ""
    }
    Assert-True ($appLog -match 'tab-ux event=rename-finish tab_id=1') "Rename Tab command completed before preset load"
    Add-Trace "settings verified"
} finally {
    if ($null -ne $app -and -not $app.HasExited) {
        [J3GridDockerMenuSmoke.Native]::PostMessage($app.MainWindowHandle, $WmCommand, [IntPtr]$CmdWindowClose, [IntPtr]::Zero) | Out-Null
        $app.WaitForExit(3000) | Out-Null
        if (-not $app.HasExited) {
            $app.Kill()
        }
    }
    Add-Trace "trace $TracePath"
    Save-Trace
}
