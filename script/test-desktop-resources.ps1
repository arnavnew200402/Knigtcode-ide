[CmdletBinding()]
param([string]$ResourceDirectory)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repository = Split-Path -Parent $PSScriptRoot
if (-not $ResourceDirectory) { $ResourceDirectory = Join-Path $repository 'target/resources/desktop' }
$acquisitionScript = Join-Path $PSScriptRoot 'acquire-desktop-resources.ps1'
$desktopResourceAcquisitionScript = $acquisitionScript
$bundleScript = Join-Path $PSScriptRoot 'bundle-windows.ps1'
$SpecificationPath = Join-Path $PSScriptRoot 'desktop-resources.json'
$specification = Get-Content -LiteralPath $SpecificationPath -Raw | ConvertFrom-Json
$DownloadCacheDirectory = $null
$ExpectedArchitecture = 'x64'

function Assert-Rejected([scriptblock]$Operation, [string]$Message) {
    try { & $Operation } catch {
        if ($_.Exception.Message -notlike "*$Message*") { throw }
        Write-Output "PASS: rejected $Message"
        return
    }
    throw "Expected rejection: $Message"
}

foreach ($path in @($acquisitionScript, $bundleScript, $PSCommandPath)) {
    $tokens = $null
    $parseErrors = $null
    $syntax = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$parseErrors)
    if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
    if ($path -eq $PSCommandPath) { continue }
    foreach ($definition in $syntax.FindAll({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst]
    }, $false)) {
        if ($path -eq $acquisitionScript -or $definition.Name -in @('VerifyDesktopResources', 'StageDesktopResources')) {
            # Load only production functions, avoiding the bundle script's Cargo and installer entry point.
            . ([scriptblock]::Create($definition.Extent.Text))
        }
    }
}

& $acquisitionScript -OutputDirectory $ResourceDirectory -VerifyOnly -ExpectedArchitecture x64
$parent = Join-Path $repository 'target/resources'
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw 'Test parent missing.' }
$temporary = Join-Path $parent ('resource-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporary | Out-Null
try {
    $Architecture = 'x86_64'
    $desktopResourcesSource = [IO.Path]::GetFullPath($ResourceDirectory)
    $desktopResourcesSpecification = $SpecificationPath
    $innoDir = Join-Path $temporary 'inno'
    New-Item -ItemType Directory -Path $innoDir | Out-Null
    Assert-Rejected { VerifyDesktopResources (Join-Path $temporary 'missing') } 'resources are missing'
    StageDesktopResources
    $fixture = Join-Path $innoDir 'resources/desktop'
    $manifestPath = Join-Path $fixture 'manifest.json'
    $originalManifest = [IO.File]::ReadAllText($manifestPath)
    $manifest = $originalManifest | ConvertFrom-Json
    $runtime = Join-Path $fixture 'webview2/msedgewebview2.exe'
    Assert-PeArchitecture $runtime 'x64'
    Assert-Rejected { Assert-PeArchitecture $runtime 'arm64' } 'architecture mismatch'
    Assert-MicrosoftSignature $runtime $specification.webview2.authenticode_thumbprint
    Assert-Rejected { Assert-MicrosoftSignature $runtime ('0' * 40) } 'Authenticode verification failed'
    $identities = @((Get-Acl -LiteralPath (Join-Path $fixture 'webview2')).GetAccessRules(
        $true, $true, [Security.Principal.SecurityIdentifier]) | ForEach-Object {
        $_.IdentityReference.Value
    })
    foreach ($identity in @('S-1-15-2-1', 'S-1-15-2-2')) {
        if ($identity -notin $identities) { throw "Missing staged runtime ACL: $identity" }
    }
    Write-Output 'PASS: real recursive staging, Microsoft signature, architecture and AppContainer ACLs'

    $notice = Join-Path $fixture 'notices/Whisper.txt'
    [IO.File]::WriteAllText($notice, 'deliberately corrupted license test input')
    Assert-Rejected { Assert-ResourceBundle $fixture } 'SHA-256 mismatch'
    Copy-Item -LiteralPath (Join-Path $ResourceDirectory 'notices/Whisper.txt') -Destination $notice -Force
    $model = Join-Path $fixture 'speech/ggml-tiny.en.bin'
    $stream = [IO.File]::OpenWrite($model)
    try { $stream.WriteByte(0) } finally { $stream.Dispose() }
    Assert-Rejected { Assert-ResourceBundle $fixture } 'SHA-256 mismatch'
    Copy-Item -LiteralPath (Join-Path $ResourceDirectory 'speech/ggml-tiny.en.bin') -Destination $model -Force
    $manifest.webview2.version = '0.0.0.0'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10))
    Assert-Rejected { Assert-ResourceBundle $fixture } 'metadata differs'
    [IO.File]::WriteAllText($manifestPath, $originalManifest)
    $manifest = $originalManifest | ConvertFrom-Json
    $manifest.files += [pscustomobject]@{ path = '../escaped'; bytes = 1; sha256 = $specification.speech.sha256 }
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10))
    Assert-Rejected { Assert-ResourceBundle $fixture } 'path escapes bundle'
    [IO.File]::WriteAllText($manifestPath, $originalManifest)

    $extra = Join-Path $fixture 'untracked.txt'
    [IO.File]::WriteAllText($extra, 'untracked test input')
    Assert-Rejected { Assert-ResourceBundle $fixture } 'Untracked file'
    Remove-Item -LiteralPath $extra
    $manifest = $originalManifest | ConvertFrom-Json
    $locale = $manifest.files | Where-Object { $_.path -like 'webview2/Locales/*.pak' } | Select-Object -First 1
    if (-not $locale) { throw 'No vendor locale file available for inventory test.' }
    $localePath = Join-Path $fixture $locale.path
    Remove-Item -LiteralPath $localePath
    $manifest.files = @($manifest.files | Where-Object { $_.path -ne $locale.path })
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 10))
    Assert-Rejected { Assert-ResourceBundle $fixture } 'inventory differs'
    Copy-Item -LiteralPath (Join-Path $ResourceDirectory $locale.path) -Destination $localePath
    [IO.File]::WriteAllText($manifestPath, $originalManifest)
    Assert-ResourceBundle $fixture

    Assert-Rejected { Get-VerifiedResource @{url='http://example.invalid'; sha256=$specification.speech.sha256} `
        (Join-Path $temporary 'unused') } 'HTTPS'
    foreach ($path in @('../escape', '/absolute', 'C:/absolute', 'file:stream', 'folder/../escape', 'folder\escape')) {
        Assert-Rejected { Assert-RelativePath $path } 'path escapes bundle'
    }
    $archivePath = Join-Path $temporary 'traversal.zip'
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::Open($archivePath, [IO.Compression.ZipArchiveMode]::Create)
    try { $archive.CreateEntry('../escape') | Out-Null } finally { $archive.Dispose() }
    Assert-Rejected { Expand-VerifiedZip $archivePath (Join-Path $temporary 'extracted') } 'path escapes bundle'
    if (Test-Path -LiteralPath (Join-Path $temporary 'extracted')) { throw 'Unsafe ZIP was extracted.' }

    $cache = Join-Path $temporary 'cache'
    New-Item -ItemType Directory -Path $cache | Out-Null
    [IO.File]::WriteAllText((Join-Path $cache $specification.webview2.sha256), 'corrupted archive cache test input')
    $acquisitionOutput = Join-Path $temporary 'acquisition'
    Assert-Rejected { & $acquisitionScript -OutputDirectory $acquisitionOutput -DownloadCacheDirectory $cache } 'SHA-256 mismatch'
    if (Test-Path -LiteralPath $acquisitionOutput) { throw 'Failed acquisition published a bundle.' }
    if (@(Get-ChildItem -LiteralPath $temporary -Directory -Filter '.desktop-resources-*').Count) {
        throw 'Failed acquisition left staging files.'
    }
    Assert-Rejected { & $acquisitionScript -OutputDirectory (Join-Path $temporary 'no-parent/output') } 'parent does not exist'

    $installer = Get-Content -LiteralPath (Join-Path $repository 'crates/zed/resources/windows/zed.iss') -Raw
    if ($installer -notmatch 'Source: "\{#ResourcesDir\}\\resources\\\*"; DestDir: "\{code:GetInstallDir\}\\resources"; Flags: ignoreversion recursesubdirs createallsubdirs') {
        throw 'Installer does not recursively stage resources through the update-aware install path.'
    }
    $updater = Get-Content -LiteralPath (Join-Path $repository 'crates/auto_update_helper/src/updater.rs') -Raw
    $backup = $updater.IndexOf('Job::move_if_exists(p("resources"), p("old\\resources"))')
    $install = $updater.IndexOf('Job::move_file(p("install\\resources"), p("resources"))')
    $cleanup = $updater.IndexOf('Job::rmdir_nofail(p("install"))')
    if ($backup -lt 0 -or $install -le $backup -or $cleanup -le $install) { throw 'Updater resource swap is missing or ordered after cleanup.' }
    $bundle = Get-Content -LiteralPath $bundleScript -Raw
    if ($bundle.Contains('$env:WHISPER_DONT_GENERATE_BINDINGS = "1"')) {
        throw 'Windows packaging must not select the Linux-generated Whisper bindings.'
    }
    foreach ($setting in @('$env:GGML_NATIVE = "OFF"', 'Set-BindgenTarget $target', '$env:LIBCLANG_PATH = $libclang')) {
        if (-not $bundle.Contains($setting)) { throw "Windows Whisper build setting missing: $setting" }
    }
    Write-Output 'PASS: installer/update resource-path contract and portable native build settings'
} finally { Remove-Item -LiteralPath $temporary -Recurse -Force }

Write-Output 'PASS: desktop resource acquisition and Windows packaging integrity tests'
