[CmdletBinding(DefaultParameterSetName = 'Acquire')]
param(
    [string]$SpecificationPath,
    [Parameter(Mandatory = $true)][string]$OutputDirectory,
    [Parameter(Mandatory = $true, ParameterSetName = 'Verify')][switch]$VerifyOnly,
    [Parameter(ParameterSetName = 'Acquire')][string]$DownloadCacheDirectory,
    [ValidateSet('x64', 'arm64')][string]$ExpectedArchitecture
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $SpecificationPath) { $SpecificationPath = Join-Path $PSScriptRoot 'desktop-resources.json' }

function Assert-Digest([string]$Digest) {
    if ($Digest -notmatch '^[a-fA-F0-9]{64}$') { throw 'Every resource and license must have an explicitly reviewed SHA-256 digest.' }
}
function Assert-Checksum([string]$Path, [string]$Digest) {
    Assert-Digest $Digest
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Resource missing: $Path" }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($actual -ine $Digest) { throw "SHA-256 mismatch for $Path. Expected $Digest, received $actual" }
}
function Assert-RelativePath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or $Path -match '(^[/\\]|[:\\]|(^|/)\.{1,2}(/|$)|[. ](/|$))') {
        throw "Resource path escapes bundle or is not canonical: $Path"
    }
}
function Get-VerifiedResource($Resource, [string]$Path) {
    Assert-Digest $Resource.sha256
    $url = [Uri]$Resource.url
    if (-not $url.IsAbsoluteUri -or $url.Scheme -ne 'https') { throw 'Resource URLs must use HTTPS.' }
    if ($url.UserInfo -or $url.Fragment) { throw 'Resource URLs must not contain credentials or fragments.' }
    $cached = if ($DownloadCacheDirectory) { Join-Path $DownloadCacheDirectory $Resource.sha256 } else { $null }
    if ($cached -and (Test-Path -LiteralPath $cached -PathType Leaf)) {
        Assert-Checksum $cached $Resource.sha256
        Copy-Item -LiteralPath $cached -Destination $Path
    } else {
        Invoke-WebRequest -Uri $url.AbsoluteUri -OutFile $Path -UseBasicParsing
    }
    Assert-Checksum $Path $Resource.sha256
    if ((Get-Item -LiteralPath $Path).Length -ne $Resource.bytes) { throw "Resource size mismatch: $Path" }
    if ($cached -and -not (Test-Path -LiteralPath $cached)) { Copy-Item -LiteralPath $Path -Destination $cached }
}
function Get-VerifiedNotice($Notice, [string]$Path, [string]$Downloads) {
    $source = Join-Path $Downloads $Notice.sha256
    if (-not (Test-Path -LiteralPath $source)) { Get-VerifiedResource $Notice $source }
    Assert-Checksum $source $Notice.sha256
    switch ($Notice.format) {
        'text' { Copy-Item -LiteralPath $source -Destination $Path }
        'zip-entry' {
            Add-Type -AssemblyName System.IO.Compression.FileSystem
            $archive = [IO.Compression.ZipFile]::OpenRead($source)
            try {
                $entries = @($archive.Entries | Where-Object { $_.FullName -ceq $Notice.entry })
                if ($entries.Count -ne 1) { throw "Notice archive entry missing or duplicated: $($Notice.entry)" }
                $input = $entries[0].Open()
                $outputFile = [IO.File]::Create($Path)
                try { $input.CopyTo($outputFile) } finally { $input.Dispose(); $outputFile.Dispose() }
            } finally { $archive.Dispose() }
        }
        default { throw "Unsupported notice format: $($Notice.format)" }
    }
    Assert-Checksum $Path $Notice.output_sha256
    if ((Get-Item -LiteralPath $Path).Length -ne $Notice.output_bytes) { throw "Notice size mismatch: $Path" }
}
function Assert-ResourceBundle([string]$Directory) {
    $manifest = Get-Content -LiteralPath (Join-Path $Directory 'manifest.json') -Raw | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or $manifest.architecture -ne $specification.architecture -or
        ($ExpectedArchitecture -and $manifest.architecture -ne $ExpectedArchitecture)) { throw 'Desktop resource architecture/schema mismatch.' }
    if ($manifest.specification_sha256 -ine (Get-FileHash -LiteralPath $SpecificationPath -Algorithm SHA256).Hash) {
        throw 'Desktop resources do not match the reviewed acquisition specification; reacquire them.'
    }
    if ($manifest.browser.windows_runtime -ne 'evergreen' -or $manifest.browser.macos_runtime -ne 'wkwebview') {
        throw 'Browser preview must use the system runtime.'
    }
    if (Test-Path -LiteralPath (Join-Path $Directory 'webview2')) { throw 'Private browser runtimes are no longer bundled.' }
    if ($manifest.speech.model -ne 'tiny.en' -or $manifest.speech.license -ne 'MIT' -or
        $manifest.speech.sha256 -ine $specification.speech.sha256 -or $manifest.speech.bytes -ne $specification.speech.bytes) {
        throw 'Desktop resource metadata differs from specification.'
    }
    $approved = @{ 'speech/ggml-tiny.en.bin' = @{ sha256 = $specification.speech.sha256; bytes = $specification.speech.bytes } }
    foreach ($notice in $specification.notices) {
        Assert-RelativePath $notice.file
        $key = 'notices/' + $notice.file
        if ($approved.ContainsKey($key)) { throw "Duplicate approved notice: $key" }
        $approved[$key] = @{ sha256 = $notice.output_sha256; bytes = $notice.output_bytes }
    }
    $root = [IO.Path]::GetFullPath($Directory) + [IO.Path]::DirectorySeparatorChar
    $tracked = @{}
    foreach ($file in $manifest.files) {
        Assert-RelativePath $file.path
        if (-not $approved.ContainsKey($file.path)) { throw "Unapproved resource: $($file.path)" }
        if ($tracked.ContainsKey($file.path)) { throw "Duplicate resource manifest entry: $($file.path)" }
        $path = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $file.path))
        if (-not $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw 'Manifest path escapes bundle.' }
        $expected = $approved[$file.path]
        if ($file.sha256 -ine $expected.sha256 -or $file.bytes -ne $expected.bytes) { throw 'Inventory differs from reviewed specification.' }
        Assert-Checksum $path $expected.sha256
        if ((Get-Item -LiteralPath $path).Length -ne $expected.bytes) { throw "Inventory size mismatch: $($file.path)" }
        $tracked[$file.path] = $true
    }
    foreach ($required in $approved.Keys) {
        if (-not $tracked.ContainsKey($required)) { throw "Required resource is missing from inventory: $required" }
    }
    foreach ($file in Get-ChildItem -LiteralPath $Directory -Recurse -Force) {
        if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Resource links are not allowed: $($file.FullName)" }
        if ($file.PSIsContainer) { continue }
        $relative = $file.FullName.Substring($root.Length).Replace('\', '/')
        if ($relative -ne 'manifest.json' -and -not $tracked.ContainsKey($relative)) { throw "Untracked file in desktop resources: $relative" }
    }
}

$output = [IO.Path]::GetFullPath($OutputDirectory)
$specification = Get-Content -LiteralPath $SpecificationPath -Raw | ConvertFrom-Json
if ($specification.schema_version -ne 1 -or $specification.architecture -notin @('x64', 'arm64') -or
    ($ExpectedArchitecture -and $ExpectedArchitecture -ne $specification.architecture)) { throw 'Acquisition specification architecture/schema mismatch.' }
if ($specification.browser.windows_runtime -ne 'evergreen' -or $specification.browser.macos_runtime -ne 'wkwebview') { throw 'Specify system browser runtimes.' }
if ($specification.speech.model -ne 'tiny.en' -or $specification.speech.license -ne 'MIT') { throw 'Specify a reviewed GGML Whisper tiny.en model with its MIT license.' }
foreach ($resource in @($specification.speech) + @($specification.notices)) {
    Assert-Digest $resource.sha256
    $url = [Uri]$resource.url
    if (-not $url.IsAbsoluteUri -or $url.Scheme -ne 'https' -or $url.UserInfo -or $url.Fragment) { throw 'Resource URLs must use HTTPS without credentials or fragments.' }
    if ($resource.bytes -le 0) { throw 'Every downloaded resource must specify its exact byte count.' }
}
foreach ($notice in $specification.notices) { Assert-RelativePath $notice.file; Assert-Digest $notice.output_sha256 }
if ($VerifyOnly) {
    Assert-ResourceBundle $output
    Write-Output "Verified bundled speech model/notices; browser uses system runtime: $output"
    return
}
if (Test-Path -LiteralPath $output) { throw "Destination exists; use -VerifyOnly or select a new destination: $output" }
$parent = Split-Path -Parent $output
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw "Destination parent does not exist: $parent" }
if ($DownloadCacheDirectory -and -not (Test-Path -LiteralPath $DownloadCacheDirectory -PathType Container)) { throw "Download cache directory does not exist: $DownloadCacheDirectory" }
$staging = Join-Path $parent ('.desktop-resources-' + [Guid]::NewGuid().ToString('N'))
$downloads = Join-Path $staging 'downloads'
$bundle = Join-Path $staging 'bundle'
try {
    New-Item -ItemType Directory -Path $downloads, $bundle, (Join-Path $bundle 'speech'), (Join-Path $bundle 'notices') | Out-Null
    Get-VerifiedResource $specification.speech (Join-Path $bundle 'speech/ggml-tiny.en.bin')
    foreach ($notice in $specification.notices) { Get-VerifiedNotice $notice (Join-Path $bundle ('notices/' + $notice.file)) $downloads }
    $root = [IO.Path]::GetFullPath($bundle) + [IO.Path]::DirectorySeparatorChar
    $files = @(Get-ChildItem -LiteralPath $bundle -Recurse -File -Force | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($root.Length).Replace('\', '/'); sha256 = (Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant(); bytes = $_.Length }
    })
    $manifest = [ordered]@{
        schema_version = 1
        architecture = $specification.architecture
        specification_sha256 = (Get-FileHash -LiteralPath $SpecificationPath).Hash.ToLowerInvariant()
        browser = $specification.browser
        speech = [ordered]@{
            model = 'tiny.en'; file = 'speech/ggml-tiny.en.bin'; url = $specification.speech.url
            sha256 = $specification.speech.sha256.ToLowerInvariant(); license = 'MIT'; bytes = $specification.speech.bytes
            revision = $specification.speech.revision; verification = $specification.speech.verification
        }
        notices = $specification.notices
        files = $files
    }
    [IO.File]::WriteAllText((Join-Path $bundle 'manifest.json'), ($manifest | ConvertTo-Json -Depth 10), [Text.UTF8Encoding]::new($false))
    Assert-ResourceBundle $bundle
    Move-Item -LiteralPath $bundle -Destination $output
    Write-Output "Acquired verified speech resources (no private browser runtime): $output"
} finally {
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
}
