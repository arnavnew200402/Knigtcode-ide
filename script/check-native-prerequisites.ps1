<#
.SYNOPSIS
Checks the native Windows x64 development toolchain without installing software.
.DESCRIPTION
Finds installed tools outside PATH, initializes the Visual Studio developer shell,
and validates the SDK headers, libraries, and tools. Restores the process environment
and caller location when invoked directly. PowerShell 5.1 is sufficient for development.
.PARAMETER ForBundle
Also require PowerShell 7 and Inno Setup 6, as used by bundle-windows.ps1.
.EXAMPLE
powershell -NoProfile -ExecutionPolicy Bypass -File script/check-native-prerequisites.ps1
.EXAMPLE
.\script\check-native-prerequisites.ps1 -ForBundle
#>
[CmdletBinding()]
param([switch]$ForBundle)

function Find-NativeExecutable {
    param([string]$Name, [string[]]$Candidates = @())

    $command = Get-Command $Name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) { return $candidate }
    }
    return $null
}

function Find-LibclangDirectory {
    param([string]$VisualStudioPath)

    $candidates = @()
    if ($env:LIBCLANG_PATH) { $candidates += $env:LIBCLANG_PATH }
    $candidates += @(
        (Join-Path $env:ProgramFiles 'LLVM\bin'),
        (Join-Path ${env:ProgramFiles(x86)} 'LLVM\bin')
    )
    if ($VisualStudioPath) {
        $candidates += @(
            (Join-Path $VisualStudioPath 'VC\Tools\Llvm\x64\bin'),
            (Join-Path $VisualStudioPath 'VC\Tools\Llvm\bin')
        )
    }

    foreach ($candidate in ($candidates | Where-Object { $_ } | Select-Object -Unique)) {
        $directory = if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            Split-Path -Parent $candidate
        } else {
            $candidate
        }
        foreach ($library in @('libclang.dll', 'clang.dll')) {
            if (Test-Path -LiteralPath (Join-Path $directory $library) -PathType Leaf) {
                return $directory.TrimEnd('\')
            }
        }
    }
    return $null
}

function Set-BindgenTarget {
    param([Parameter(Mandatory)][string]$Target)

    $targetArgument = "--target=$Target"
    if ($env:BINDGEN_EXTRA_CLANG_ARGS) {
        $targetArgument += " $env:BINDGEN_EXTRA_CLANG_ARGS"
    }
    $env:BINDGEN_EXTRA_CLANG_ARGS = $targetArgument
}

function Restore-NativeEnvironment {
    param([System.Collections.IDictionary]$Snapshot)

    foreach ($name in [Environment]::GetEnvironmentVariables('Process').Keys) {
        if (-not $Snapshot.Contains($name)) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    }
    foreach ($name in $Snapshot.Keys) {
        [Environment]::SetEnvironmentVariable($name, $Snapshot[$name], 'Process')
    }
}

function Get-NativeWindowsSdk {
    param([string[]]$Roots)

    foreach ($root in ($Roots | Where-Object { $_ } | Select-Object -Unique)) {
        $libraryDirectory = Join-Path $root 'Lib'
        if (-not (Test-Path -LiteralPath $libraryDirectory -PathType Container)) { continue }
        $versions = Get-ChildItem -LiteralPath $libraryDirectory -Directory |
            Where-Object { $_.Name -match '^10\.0\.\d+\.\d+$' -and [version]$_.Name -ge [version]'10.0.20348.0' } |
            Sort-Object { [version]$_.Name } -Descending
        foreach ($version in $versions) {
            $requiredFiles = @(
                "Lib\$($version.Name)\um\x64\kernel32.lib",
                "Lib\$($version.Name)\um\x64\user32.lib",
                "Lib\$($version.Name)\um\x64\ole32.lib",
                "Lib\$($version.Name)\ucrt\x64\ucrt.lib",
                "Lib\$($version.Name)\ucrt\x64\libucrt.lib",
                "Include\$($version.Name)\um\Windows.h",
                "Include\$($version.Name)\um\winres.h",
                "Include\$($version.Name)\shared\sdkddkver.h",
                "Include\$($version.Name)\ucrt\stdio.h",
                "Include\$($version.Name)\winrt\windows.foundation.h",
                "bin\$($version.Name)\x64\rc.exe",
                "bin\$($version.Name)\x64\mt.exe"
            )
            $missingFiles = @($requiredFiles | Where-Object {
                -not (Test-Path -LiteralPath (Join-Path $root $_) -PathType Leaf)
            })
            if ($missingFiles.Count -eq 0) {
                return [pscustomobject]@{ Root = $root.TrimEnd('\'); Version = $version.Name }
            }
            Write-Host "Incomplete Windows SDK $($version.Name) at ${root}: $($missingFiles -join ', ')"
        }
    }
    return $null
}

function Get-NativePrerequisites {
    param([string]$RepositoryRoot, [switch]$ForBundle)

    $ErrorActionPreference = 'Stop'
    $PSNativeCommandUseErrorActionPreference = $false
    if ([Environment]::OSVersion.Platform -ne 'Win32NT' -or -not [Environment]::Is64BitProcess) {
        throw 'Run this script in native 64-bit Windows PowerShell, not WSL or 32-bit PowerShell.'
    }
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw 'This development launcher currently checks the x86_64-pc-windows-msvc toolchain only.'
    }

    $checks = [System.Collections.Generic.List[object]]::new()
    $sdkRoots = @($env:WindowsSdkDir, $env:UniversalCRTSdkDir)
    $installer = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\setup.exe'
    $vswhere = Find-NativeExecutable 'vswhere.exe' @(
        (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe')
    )
    $visualStudio = $null
    if ($vswhere) {
        $installations = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -format json
        if ($LASTEXITCODE -ne 0) { throw "vswhere failed with exit code $LASTEXITCODE." }
        $visualStudio = $installations | ConvertFrom-Json |
            Sort-Object { [version]$_.installationVersion } -Descending | Select-Object -First 1
    }
    $visualStudioPath = if ($visualStudio) { $visualStudio.installationPath } else { $null }
    $newInstallRemedy = 'winget install --exact --id Microsoft.VisualStudio.2022.BuildTools --override ''--add Microsoft.VisualStudio.Workload.VCTools --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64 --add Microsoft.VisualStudio.Component.Windows11SDK.26100 --add Microsoft.VisualStudio.Component.VC.CMake.Project --add Microsoft.VisualStudio.Component.VC.Runtimes.x86.x64.Spectre --passive --norestart'' (requires Administrator/UAC).'
    $checks.Add([pscustomobject]@{
        Name = 'Visual Studio'; Required = $true; Found = [bool]$visualStudio
        Detail = $visualStudioPath; Remedy = $newInstallRemedy
    })

    $shellReady = $false
    $shellDetail = 'No installed VS developer shell with the x64 C++ tools.'
    if ($visualStudio) {
        $shell = Join-Path $visualStudioPath 'Common7\Tools\Launch-VsDevShell.ps1'
        try {
            if (-not (Test-Path -LiteralPath $shell -PathType Leaf)) { throw "Missing $shell" }
            & $shell -VsInstallationPath $visualStudioPath -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Host
            $shellReady = $env:VSCMD_ARG_TGT_ARCH -eq 'x64' -and $env:VSCMD_ARG_HOST_ARCH -eq 'x64' -and
                $env:VCToolsInstallDir -and $env:VSINSTALLDIR.TrimEnd('\') -ieq $visualStudioPath.TrimEnd('\')
            $shellDetail = if ($shellReady) { $env:VCToolsInstallDir } else { 'VS shell did not select the requested x64 tools.' }
        } catch { $shellDetail = $_.Exception.Message }
    }
    $repairRemedy = if ($visualStudio) {
        "Start-Process -FilePath '$installer' -ArgumentList 'repair --installPath `"$visualStudioPath`" --passive --norestart' -Wait -PassThru (run in Administrator PowerShell)."
    } else { $newInstallRemedy }
    $checks.Add([pscustomobject]@{
        Name = 'VS developer shell'; Required = $true; Found = [bool]$shellReady
        Detail = $shellDetail; Remedy = $repairRemedy
    })

    $libclang = Find-LibclangDirectory -VisualStudioPath $visualStudioPath
    $checks.Add([pscustomobject]@{
        Name = 'LLVM/libclang'; Required = $true; Found = [bool]$libclang; Detail = $libclang
        Remedy = 'winget install --exact --id LLVM.LLVM --version 22.1.6 --source winget --scope machine --silent --accept-package-agreements --accept-source-agreements --disable-interactivity.'
    })
    if ($libclang) { $env:LIBCLANG_PATH = $libclang }

    $compiler = $null
    $linker = $null
    $spectre = $null
    if ($shellReady) {
        $compiler = Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64\cl.exe'
        $linker = Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64\link.exe'
        $spectre = Join-Path $env:VCToolsInstallDir 'lib\spectre\x64\libcmt.lib'
        $env:PATH = "$(Split-Path -Parent $compiler);$env:PATH"
    }
    $msvcReady = $compiler -and $linker -and
        (Test-Path -LiteralPath $compiler -PathType Leaf) -and (Test-Path -LiteralPath $linker -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $env:VCToolsInstallDir 'lib\x64\libcmt.lib') -PathType Leaf) -and
        (Test-Path -LiteralPath (Join-Path $env:VCToolsInstallDir 'include\vcruntime.h') -PathType Leaf)
    $checks.Add([pscustomobject]@{
        Name = 'MSVC compiler/linker'; Required = $true; Found = [bool]$msvcReady
        Detail = $compiler; Remedy = $repairRemedy
    })
    $spectreRemedy = if ($visualStudio) {
        "Start-Process -FilePath '$installer' -ArgumentList 'modify --installPath `"$visualStudioPath`" --add Microsoft.VisualStudio.Component.VC.Runtimes.x86.x64.Spectre --passive --norestart' -Wait -PassThru (run in Administrator PowerShell)."
    } else { $newInstallRemedy }
    $checks.Add([pscustomobject]@{
        Name = 'MSVC Spectre libraries'; Required = $true
        Found = [bool]($spectre -and (Test-Path -LiteralPath $spectre -PathType Leaf))
        Detail = $spectre; Remedy = $spectreRemedy
    })

    $sdkRoots += @($env:WindowsSdkDir, $env:UniversalCRTSdkDir)
    foreach ($registryPath in @(
        'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows Kits\Installed Roots'
    )) {
        $registeredSdk = Get-ItemProperty -LiteralPath $registryPath -ErrorAction SilentlyContinue
        if ($registeredSdk.KitsRoot10) { $sdkRoots += $registeredSdk.KitsRoot10 }
    }
    $sdkRoots += Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10'
    $sdkRoots = @($sdkRoots | Where-Object { $_ } | ForEach-Object { $_.TrimEnd('\') } | Select-Object -Unique)
    $sdk = Get-NativeWindowsSdk $sdkRoots
    $sdkRemedy = $newInstallRemedy
    if ($visualStudio) {
        $registeredSdkInstall = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.Windows11SDK.26100 -property installationPath
        if ($LASTEXITCODE -ne 0) { throw "vswhere SDK query failed with exit code $LASTEXITCODE." }
        $sdkRemedy = if ($visualStudioPath -in $registeredSdkInstall) { $repairRemedy } else {
            "Start-Process -FilePath '$installer' -ArgumentList 'modify --installPath `"$visualStudioPath`" --add Microsoft.VisualStudio.Component.Windows11SDK.26100 --passive --norestart' -Wait -PassThru (run in Administrator PowerShell)."
        }
    }
    $sdkDetail = if ($sdk) { "$($sdk.Version) at $($sdk.Root)" } else {
        "No complete SDK >= 10.0.20348.0 (kernel32.lib, UCRT, headers, rc.exe, mt.exe). Searched: $($sdkRoots | Where-Object { $_ } | Select-Object -Unique)"
    }
    $checks.Add([pscustomobject]@{
        Name = 'Windows SDK'; Required = $true; Found = [bool]$sdk; Detail = $sdkDetail; Remedy = $sdkRemedy
    })
    if ($sdk) {
        $env:WindowsSdkDir = "$($sdk.Root)\"
        $env:WindowsSDKVersion = "$($sdk.Version)\"
        $env:WindowsSDKLibVersion = $env:WindowsSDKVersion
        $env:UniversalCRTSdkDir = $env:WindowsSdkDir
        $env:UCRTVersion = $sdk.Version
        $env:WindowsSdkBinPath = "$($sdk.Root)\bin\"
        $env:WindowsSdkVerBinPath = "$($sdk.Root)\bin\$($sdk.Version)\"
        $env:ZED_RC_TOOLKIT_PATH = "$($env:WindowsSdkVerBinPath)x64"
        $env:PATH = "$env:ZED_RC_TOOLKIT_PATH;$env:PATH"
        $env:LIB = "$($sdk.Root)\Lib\$($sdk.Version)\um\x64;$($sdk.Root)\Lib\$($sdk.Version)\ucrt\x64;$env:LIB"
        $sdkIncludes = @('ucrt', 'shared', 'um', 'winrt', 'cppwinrt') | ForEach-Object {
            "$($sdk.Root)\Include\$($sdk.Version)\$_"
        }
        $env:INCLUDE = "$($sdkIncludes -join ';');$env:INCLUDE"
    }

    $cmakeCandidates = @((Join-Path $env:ProgramFiles 'CMake\bin\cmake.exe'))
    if ($visualStudio) {
        $cmakeCandidates += Join-Path $visualStudioPath 'Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe'
    }
    $cmake = Find-NativeExecutable 'cmake.exe' $cmakeCandidates
    $cmakeVersion = $null
    $cmakeReady = $false
    if ($cmake) {
        $cmakeOutput = & $cmake --version
        $cmakeReady = $LASTEXITCODE -eq 0
        $cmakeVersion = $cmakeOutput | Select-Object -First 1
        $env:PATH = "$(Split-Path -Parent $cmake);$env:PATH"
    }
    $checks.Add([pscustomobject]@{
        Name = 'CMake'; Required = $true; Found = $cmakeReady; Detail = "$cmakeVersion $cmake"
        Remedy = 'winget install --exact --id Kitware.CMake; or add Microsoft.VisualStudio.Component.VC.CMake.Project in Visual Studio Installer (machine installation requires Administrator/UAC).'
    })

    $channelMatch = [regex]::Match((Get-Content -LiteralPath (Join-Path $RepositoryRoot 'rust-toolchain.toml') -Raw), '(?m)^\s*channel\s*=\s*"([^"]+)"')
    if (-not $channelMatch.Success) { throw 'Cannot read the pinned channel from rust-toolchain.toml.' }
    $channel = $channelMatch.Groups[1].Value
    $toolchain = "$channel-x86_64-pc-windows-msvc"
    $rustup = Find-NativeExecutable 'rustup.exe' @((Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'))
    $rustReady = $false
    $rustDetail = "Missing installed $toolchain toolchain."
    if ($rustup) {
        $env:PATH = "$(Split-Path -Parent $rustup);$env:PATH"
        $installedToolchains = & $rustup toolchain list
        if ($LASTEXITCODE -ne 0) { throw "rustup toolchain list failed with exit code $LASTEXITCODE." }
        if ($installedToolchains -match ('^' + [regex]::Escape($toolchain) + '(\s|$)')) {
            $rustVersion = & $rustup run $toolchain rustc -vV
            $rustReady = $LASTEXITCODE -eq 0 -and $rustVersion -contains 'host: x86_64-pc-windows-msvc' -and
                $rustVersion -contains "release: $channel"
            $cargoVersion = & $rustup run $toolchain cargo --version
            $rustReady = $rustReady -and $LASTEXITCODE -eq 0
            $rustDetail = "$($rustVersion | Select-Object -First 1); $cargoVersion; $rustup"
        }
    }
    $checks.Add([pscustomobject]@{
        Name = 'Pinned Rust/Cargo'; Required = $true; Found = [bool]$rustReady; Detail = $rustDetail
        Remedy = "Install rustup from https://rustup.rs if absent, then: rustup toolchain install $toolchain --profile minimal --component rustfmt,clippy,rust-analyzer,rust-src (per-user; no Administrator required)."
    })
    $git = Find-NativeExecutable 'git.exe' @((Join-Path $env:ProgramFiles 'Git\cmd\git.exe'))
    if ($git) { $env:PATH = "$(Split-Path -Parent $git);$env:PATH" }
    $checks.Add([pscustomobject]@{
        Name = 'Git'; Required = $true; Found = [bool]$git; Detail = $git
        Remedy = 'winget install --exact --id Git.Git (machine installation requires Administrator/UAC).'
    })
    $checks.Add([pscustomobject]@{
        Name = 'Repository Rust flags'; Required = $true
        Found = -not ($env:RUSTFLAGS -or $env:CARGO_ENCODED_RUSTFLAGS)
        Detail = 'RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS must be unset so .cargo/config.toml supplies the static CRT flags.'
        Remedy = 'Remove-Item Env:RUSTFLAGS,Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue'
    })

    $powerShellCandidates = @()
    $powerShellDirectory = Join-Path $env:ProgramFiles 'PowerShell'
    if (Test-Path -LiteralPath $powerShellDirectory -PathType Container) {
        $powerShellCandidates += Get-ChildItem -LiteralPath $powerShellDirectory -Directory | ForEach-Object {
            Join-Path $_.FullName 'pwsh.exe'
        }
    }
    foreach ($version in (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\PowerShellCore\InstalledVersions\*' -ErrorAction SilentlyContinue)) {
        if ($version.InstallLocation) { $powerShellCandidates += Join-Path $version.InstallLocation 'pwsh.exe' }
    }
    $powerShellCandidates += @(
        (Join-Path $env:LOCALAPPDATA 'Microsoft\PowerShell\7\pwsh.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\PowerShell\7\pwsh.exe')
    )
    $powerShell = Find-NativeExecutable 'pwsh.exe' $powerShellCandidates
    $powerShellVersion = $null
    if ($powerShell) {
        $powerShellVersion = & $powerShell -NoLogo -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'
        if ($LASTEXITCODE -ne 0) { $powerShellVersion = $null }
    }
    $checks.Add([pscustomobject]@{
        Name = 'PowerShell 7 (bundle)'; Required = [bool]$ForBundle
        Found = [bool]($powerShellVersion -and [version]$powerShellVersion -ge [version]'7.0')
        Detail = "$powerShellVersion $powerShell"
        Remedy = 'winget install --exact --id Microsoft.PowerShell (machine installation requires Administrator/UAC).'
    })
    $innoCandidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    foreach ($uninstallRoot in @(
        'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
        'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
    )) {
        Get-ItemProperty $uninstallRoot -ErrorAction SilentlyContinue | Where-Object {
            $_.DisplayName -like 'Inno Setup*' -and $_.InstallLocation
        } | ForEach-Object { $innoCandidates += Join-Path $_.InstallLocation 'ISCC.exe' }
    }
    $inno = Find-NativeExecutable 'ISCC.exe' $innoCandidates
    $innoReady = $false
    if ($inno) {
        # ISCC 6 sends help to stderr and returns 1 even when the executable works.
        $startInfo = [Diagnostics.ProcessStartInfo]::new()
        $startInfo.FileName = $inno
        $startInfo.Arguments = '/?'
        $startInfo.UseShellExecute = $false
        $startInfo.CreateNoWindow = $true
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $startInfo
        try {
            if (-not $process.Start()) { throw "Could not start $inno" }
            $standardOutput = $process.StandardOutput.ReadToEndAsync()
            $standardError = $process.StandardError.ReadToEndAsync()
            $process.WaitForExit()
            $innoHelp = $standardOutput.Result + $standardError.Result
            $innoReady = $process.ExitCode -in @(0, 1) -and $innoHelp -match '(?m)^Inno Setup 6 Command-Line Compiler'
        } finally {
            $process.Dispose()
        }
    }
    $checks.Add([pscustomobject]@{
        Name = 'Inno Setup 6 (bundle)'; Required = [bool]$ForBundle; Found = [bool]$innoReady
        Detail = $inno
        Remedy = 'winget install --exact --id JRSoftware.InnoSetup --scope user (per-user; no Administrator required).'
    })

    foreach ($check in $checks) {
        $status = if ($check.Found) { 'OK' } elseif ($check.Required) { 'MISSING' } else { 'OPTIONAL' }
        Write-Host "[$status] $($check.Name): $($check.Detail)"
        if (-not $check.Found) { Write-Host "  Remedy: $($check.Remedy)" }
    }
    $drive = Get-PSDrive -Name ([IO.Path]::GetPathRoot($RepositoryRoot).TrimEnd('\', ':')) -ErrorAction SilentlyContinue
    if ($drive) { Write-Host ('Repository disk free: {0:N1} GiB' -f ($drive.Free / 1GB)) }
    return [pscustomobject]@{
        Ready = @($checks | Where-Object { $_.Required -and -not $_.Found }).Count -eq 0
        Checks = $checks.ToArray(); Rustup = $rustup; Toolchain = $toolchain
        VisualStudio = $visualStudioPath; Sdk = $sdk; CMake = $cmake; Libclang = $libclang
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    $environmentSnapshot = [Environment]::GetEnvironmentVariables('Process')
    $exitCode = 1
    Push-Location
    try {
        $repositoryRoot = Split-Path -Parent $PSScriptRoot
        Set-Location -LiteralPath $repositoryRoot
        $result = Get-NativePrerequisites -RepositoryRoot $repositoryRoot -ForBundle:$ForBundle
        if ($result.Ready) { $exitCode = 0 }
    } catch {
        Write-Error $_ -ErrorAction Continue
    } finally {
        Restore-NativeEnvironment $environmentSnapshot
        Pop-Location
    }
    exit $exitCode
}
