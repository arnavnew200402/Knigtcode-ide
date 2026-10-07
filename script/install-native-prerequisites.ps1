<#
.SYNOPSIS
Repairs the Windows SDK and installs LLVM/libclang, Spectre libraries, PowerShell 7, and Inno Setup 6.
.DESCRIPTION
Uses the installed Visual Studio Installer and version-pinned packages from the
official winget source. Does not restart Windows. Logs installer exit codes,
transcripts, and Visual Studio setup logs under target/native-prerequisites-*.
.PARAMETER Start
Start setup in the background. Elevation uses the normal Windows UAC prompt.
Check status.json in the reported log directory; AwaitingElevation is not success.
.PARAMETER LogDirectory
An absolute Windows log directory with an existing parent.
.PARAMETER TimeoutMinutes
Maximum wait per installer. A timeout reports the PID without killing an installer.
.EXAMPLE
.\script\install-native-prerequisites.ps1 -Start
#>
[CmdletBinding()]
param(
    [switch]$Start,
    [switch]$RequestElevation,
    [string]$LogDirectory,
    [ValidateRange(1, 120)][int]$TimeoutMinutes = 40
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ([Environment]::OSVersion.Platform -ne 'Win32NT' -or -not [Environment]::Is64BitProcess) {
    throw 'Run setup in native 64-bit Windows PowerShell.'
}
$repositoryRoot = Split-Path -Parent $PSScriptRoot
if (-not $LogDirectory) {
    $LogDirectory = Join-Path $repositoryRoot ('target\native-prerequisites-' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$LogDirectory = [IO.Path]::GetFullPath($LogDirectory)
$parent = Split-Path -Parent $LogDirectory
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw "Log parent does not exist: $parent" }
New-Item -ItemType Directory -Path $LogDirectory -Force | Out-Null
$statusPath = Join-Path $LogDirectory 'status.json'
$steps = [System.Collections.Generic.List[object]]::new()
$rebootRequired = $false
$setupStarted = Get-Date

function Write-SetupStatus {
    param([string]$State, [string]$Message)

    $status = [ordered]@{
        state = $State; message = $Message; pid = $PID
        updated_at = (Get-Date).ToString('o'); reboot_required = $script:rebootRequired
        steps = $script:steps.ToArray()
    }
    [IO.File]::WriteAllText($statusPath, ($status | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    Write-Host "${State}: $Message"
}

$windowsPowerShell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
$workerArguments = "-NoLogo -NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -LogDirectory `"$LogDirectory`" -TimeoutMinutes $TimeoutMinutes"
if ($Start) {
    if (Test-Path -LiteralPath $statusPath) { throw "A setup status already exists at $statusPath. Choose a new log directory." }
    Write-SetupStatus 'Starting' 'Launching the elevation requester.'
    $requester = Start-Process -FilePath $windowsPowerShell -ArgumentList "$workerArguments -RequestElevation" -PassThru `
        -RedirectStandardOutput (Join-Path $LogDirectory 'elevation.stdout.log') `
        -RedirectStandardError (Join-Path $LogDirectory 'elevation.stderr.log')
    Write-Host "Setup requester PID: $($requester.Id). Logs: $LogDirectory"
    Write-Host 'Approve the Windows UAC prompt. If status remains AwaitingElevation after three minutes, report it before retrying.'
    exit 0
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
$elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $elevated) {
    Write-SetupStatus 'AwaitingElevation' 'Waiting for the user to approve Windows UAC.'
    try {
        $worker = Start-Process -FilePath $windowsPowerShell -ArgumentList $workerArguments -Verb RunAs -PassThru
        [IO.File]::WriteAllText((Join-Path $LogDirectory 'elevated-worker.pid'), [string]$worker.Id)
        Write-Host "Elevated setup PID: $($worker.Id). Logs: $LogDirectory"
        exit 0
    } catch {
        Write-SetupStatus 'Failed' "Elevation denied or failed: $($_.Exception.Message)"
        Write-Error $_ -ErrorAction Continue
        exit 1
    }
}
if ($RequestElevation) { throw 'RequestElevation is only for the unelevated launcher.' }

function Invoke-SetupProcess {
    param([string]$Name, [string]$Executable, [string]$Arguments)

    Write-SetupStatus 'Running' "$Name"
    Write-Host "Executable: $Executable"
    Write-Host "Arguments: $Arguments"
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru -WorkingDirectory $repositoryRoot `
        -RedirectStandardOutput (Join-Path $LogDirectory "$Name.stdout.log") `
        -RedirectStandardError (Join-Path $LogDirectory "$Name.stderr.log")
    try {
        # PowerShell 5.1 otherwise loses ExitCode after WaitForExit disposes the uncached handle.
        $process.Handle | Out-Null
        if (-not $process.WaitForExit($TimeoutMinutes * 60 * 1000)) {
            throw "$Name exceeded $TimeoutMinutes minutes. Installer PID $($process.Id) is still running; do not start another installer."
        }
        $process.WaitForExit()
        $code = $process.ExitCode
        $script:steps.Add([pscustomobject]@{
            name = $Name; executable = $Executable; arguments = $Arguments
            exit_code = $code; exit_code_hex = ('0x' + $code.ToString('X8'))
        })
        Write-Host "$Name exit code: $code (0x$($code.ToString('X8')))"
        if ($code -eq 3010) {
            $script:rebootRequired = $true
            Write-Host 'Installer requests a later reboot; setup will not restart Windows.'
        } elseif ($code -ne 0) {
            throw "$Name failed with exit code $code (0x$($code.ToString('X8'))). See $LogDirectory."
        }
    } finally {
        $process.Dispose()
    }
}

$mutex = [Threading.Mutex]::new($false, 'Global\KnightCodeNativePrerequisiteSetup')
$ownsMutex = $false
$transcriptStarted = $false
$exitCode = 1
try {
    $ownsMutex = $mutex.WaitOne(0)
    if (-not $ownsMutex) { throw 'Another KnightCode native prerequisite setup is already running.' }
    Start-Transcript -Path (Join-Path $LogDirectory 'setup.transcript.log') | Out-Null
    $transcriptStarted = $true
    Write-SetupStatus 'Running' 'Checking installed Visual Studio and winget.'
    $installerDirectory = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer'
    $installer = Join-Path $installerDirectory 'setup.exe'
    $vswhere = Join-Path $installerDirectory 'vswhere.exe'
    if (-not (Test-Path -LiteralPath $installer -PathType Leaf) -or -not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw 'Installed Visual Studio setup.exe/vswhere.exe is missing. Install VS 2022 Build Tools first.'
    }
    $visualStudio = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if ($LASTEXITCODE -ne 0 -or -not $visualStudio) { throw 'vswhere did not find installed x64 C++ Build Tools.' }
    $winget = (Get-Command winget.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source

    . "$PSScriptRoot/check-native-prerequisites.ps1"
    $sdkRoots = @((Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'))
    foreach ($registryPath in @(
        'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots'
    )) {
        $registeredSdk = Get-ItemProperty -LiteralPath $registryPath -ErrorAction SilentlyContinue
        if ($registeredSdk.KitsRoot10) { $sdkRoots += $registeredSdk.KitsRoot10 }
    }
    if (-not (Get-NativeWindowsSdk $sdkRoots)) {
        Invoke-SetupProcess 'visual-studio-repair' $installer "repair --installPath `"$visualStudio`" --passive --norestart"
        if (-not (Get-NativeWindowsSdk $sdkRoots)) {
            throw 'Visual Studio repair completed but a complete Windows SDK is still missing. Inspect the copied dd_* logs.'
        }
    }

    $toolVersion = (Get-Content -LiteralPath (Join-Path $visualStudio 'VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt') -Raw).Trim()
    $spectreLibrary = Join-Path $visualStudio "VC\Tools\MSVC\$toolVersion\lib\spectre\x64\libcmt.lib"
    if (-not (Test-Path -LiteralPath $spectreLibrary -PathType Leaf)) {
        Invoke-SetupProcess 'visual-studio-spectre' $installer "modify --installPath `"$visualStudio`" --add Microsoft.VisualStudio.Component.VC.Runtimes.x86.x64.Spectre --passive --norestart"
    }

    $packages = @(
        @{ Name = 'llvm'; Id = 'LLVM.LLVM'; Version = '22.1.6'; Architecture = 'x64' },
        @{ Name = 'powershell'; Id = 'Microsoft.PowerShell'; Version = '7.6.6.0'; Architecture = 'x64' },
        @{ Name = 'inno-setup'; Id = 'JRSoftware.InnoSetup'; Version = '6.7.3'; Architecture = 'x86' }
    )
    foreach ($package in $packages) {
        $installedVersion = $null
        if ($package.Name -eq 'llvm') {
            $libclang = Join-Path ${env:ProgramFiles} 'LLVM\bin\libclang.dll'
            if (Test-Path -LiteralPath $libclang -PathType Leaf) {
                $productVersion = (Get-Item -LiteralPath $libclang).VersionInfo.ProductVersion
                if ($productVersion -like "$($package.Version)*") { $installedVersion = $package.Version }
            }
        } elseif ($package.Name -eq 'powershell') {
            $installedPowerShell = Join-Path $env:ProgramFiles 'PowerShell\7\pwsh.exe'
            if (Test-Path -LiteralPath $installedPowerShell -PathType Leaf) {
                $versionOutput = & $installedPowerShell -NoLogo -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'
                if ($LASTEXITCODE -eq 0) { $installedVersion = $versionOutput + '.0' }
            }
        } else {
            $registeredInno = Get-ItemProperty -LiteralPath 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1' -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe') -PathType Leaf) {
                $installedVersion = $registeredInno.DisplayVersion
            }
        }
        if ($installedVersion -eq $package.Version) {
            Write-Host "$($package.Id) $installedVersion is already installed; skipping installation."
            continue
        }
        $arguments = "install --exact --id $($package.Id) --version $($package.Version) --source winget --scope machine --architecture $($package.Architecture) --silent --accept-package-agreements --accept-source-agreements --disable-interactivity --log `"$(Join-Path $LogDirectory ($package.Name + '.winget.log'))`""
        if ($package.Name -eq 'powershell') { $arguments += ' --custom "REBOOT=ReallySuppress"' }
        Invoke-SetupProcess $package.Name $winget $arguments
    }

    $postInstallPrerequisites = Get-NativePrerequisites -RepositoryRoot $repositoryRoot -ForBundle
    if (-not $postInstallPrerequisites.Ready) { throw 'Post-install native prerequisite verification failed.' }
    Write-SetupStatus 'Complete' 'Native development and bundle prerequisites verified.'
    $exitCode = 0
} catch {
    Write-SetupStatus 'Failed' $_.Exception.Message
    Write-Error $_ -ErrorAction Continue
} finally {
    if ($ownsMutex) {
        $visualStudioLogDirectory = Join-Path $LogDirectory 'visual-studio-logs'
        New-Item -ItemType Directory -Path $visualStudioLogDirectory -Force | Out-Null
        Get-ChildItem -LiteralPath ([IO.Path]::GetTempPath()) -File -Filter 'dd_*' -ErrorAction SilentlyContinue |
            Where-Object { $_.LastWriteTime -ge $setupStarted } |
            Copy-Item -Destination $visualStudioLogDirectory -ErrorAction Continue
        $mutex.ReleaseMutex()
    }
    $mutex.Dispose()
    if ($transcriptStarted) { Stop-Transcript | Out-Null }
}
exit $exitCode
