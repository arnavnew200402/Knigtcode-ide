<#
.SYNOPSIS
Runs Cargo from the repository in a checked native Windows x64 developer environment.
.DESCRIPTION
Defaults to cargo run -p zed. Uses the pinned Rust toolchain, initializes Visual Studio,
selects a complete Windows SDK, and limits development builds to four jobs without
debug information or incremental compilation. All environment changes are process-local
and restored on success or failure, together with the caller's location.
.PARAMETER CargoArguments
The Cargo command and its arguments. Use an explicit string array to preserve flags.
.PARAMETER PreflightOnly
Check prerequisites without running Cargo.
.EXAMPLE
.\script\develop-windows.ps1 -PreflightOnly
.EXAMPLE
.\script\develop-windows.ps1 -CargoArguments @('check', '-p', 'knightcode_engine')
.EXAMPLE
.\script\develop-windows.ps1 -CargoArguments @('run', '-p', 'zed', '--', 'C:\project with spaces')
#>
[CmdletBinding()]
param(
    [string[]]$CargoArguments = @('run', '-p', 'zed'),
    [switch]$PreflightOnly
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
. "$PSScriptRoot/check-native-prerequisites.ps1"

$environmentSnapshot = [Environment]::GetEnvironmentVariables('Process')
$exitCode = 1
Push-Location
try {
    $repositoryRoot = Split-Path -Parent $PSScriptRoot
    Set-Location -LiteralPath $repositoryRoot
    $prerequisites = Get-NativePrerequisites -RepositoryRoot $repositoryRoot
    if (-not $prerequisites.Ready) { throw 'Native prerequisites are incomplete. Apply the reported remedies and rerun -PreflightOnly.' }
    Set-BindgenTarget 'x86_64-pc-windows-msvc'

    $env:CARGO_BUILD_JOBS = '4'
    $env:CARGO_PROFILE_DEV_DEBUG = '0'
    $env:CARGO_PROFILE_TEST_DEBUG = '0'
    $env:CARGO_PROFILE_DEV_INCREMENTAL = 'false'
    $env:CARGO_PROFILE_TEST_INCREMENTAL = 'false'
    $env:CARGO_INCREMENTAL = '0'
    $env:CMAKE_BUILD_PARALLEL_LEVEL = '4'
    $env:ZED_RELEASE_CHANNEL = 'dev'
    $env:RELEASE_CHANNEL = 'dev'
    # A development run must use its own instance lock and not enable the bundled updater.
    [Environment]::SetEnvironmentVariable('ZED_BUNDLE', $null, 'Process')
    Write-Host 'Development environment: jobs=4, debug=0, incremental=0, release channel=dev.'

    if ($PreflightOnly) {
        $exitCode = 0
    } else {
        if ($CargoArguments.Count -eq 0 -or [string]::IsNullOrWhiteSpace($CargoArguments[0])) {
            throw 'CargoArguments must include a Cargo command.'
        }
        Write-Host "Running cargo $($CargoArguments -join ' ') from $repositoryRoot"
        & $prerequisites.Rustup run $prerequisites.Toolchain cargo @CargoArguments
        $exitCode = $LASTEXITCODE
    }
} catch {
    Write-Error $_ -ErrorAction Continue
} finally {
    Restore-NativeEnvironment $environmentSnapshot
    Pop-Location
}
exit $exitCode
