[CmdletBinding(DefaultParameterSetName = 'Acquire')]
param(
    [string]$SpecificationPath,
    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,
    [Parameter(Mandatory = $true, ParameterSetName = 'Verify')]
    [switch]$VerifyOnly,
    [Parameter(ParameterSetName = 'Acquire')]
    [string]$DownloadCacheDirectory,
    [ValidateSet('x64', 'arm64')]
    [string]$ExpectedArchitecture
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $SpecificationPath) { $SpecificationPath = Join-Path $PSScriptRoot 'desktop-resources.json' }

function Assert-Digest([string]$Digest) {
    if ($Digest -notmatch '^[a-fA-F0-9]{64}$') {
        throw 'Every resource and license must have an explicitly reviewed SHA-256 digest.'
    }
}

function Assert-Checksum([string]$Path, [string]$Digest) {
    Assert-Digest $Digest
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Resource missing: $Path" }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
    if ($actual -ine $Digest) { throw "SHA-256 mismatch for $Path. Expected $Digest, received $actual" }
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

function Assert-RelativePath([string]$Path) {
    if ([string]::IsNullOrWhiteSpace($Path) -or $Path -match '(^[/\\]|[:\\]|(^|/)\.{1,2}(/|$)|[. ](/|$))') {
        throw "Resource path escapes bundle or is not canonical: $Path"
    }
}

function Assert-MicrosoftSignature([string]$Path, [string]$Thumbprint) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate -or
        $signature.SignerCertificate.Thumbprint -ine $Thumbprint -or
        $signature.SignerCertificate.Subject -notmatch '(^|, )O=Microsoft Corporation(,|$)') {
        throw "Microsoft Authenticode verification failed for $Path ($($signature.Status))."
    }
}

function Get-VerifiedNotice($Notice, [string]$Path, [string]$Downloads) {
    $source = Join-Path $Downloads $Notice.sha256
    if (-not (Test-Path -LiteralPath $source)) { Get-VerifiedResource $Notice $source }
    Assert-Checksum $source $Notice.sha256
    switch ($Notice.format) {
        'text' { Copy-Item -LiteralPath $source -Destination $Path }
        'webview2-fixed-eula-json' {
            $terms = (Get-Content -LiteralPath $source -Raw -Encoding UTF8 | ConvertFrom-Json).fixedHtml
            if ($terms -notmatch 'MICROSOFT EDGE WEBVIEW2 RUNTIME \(FIXED VERSION\)') { throw 'Fixed runtime terms missing.' }
            $terms = [regex]::Replace($terms, '\s+', ' ')
            $terms = [regex]::Replace($terms, '</(?:p|h[1-6]|li|div)>', "`n`n")
            $terms = [regex]::Replace($terms, '<br\s*/?>', "`n")
            $terms = [Net.WebUtility]::HtmlDecode([regex]::Replace($terms, '<[^>]+>', ''))
            $paragraphs = @($terms.Split("`n") | ForEach-Object { $_.Trim() } | Where-Object { $_ })
            [IO.File]::WriteAllText($Path, ($paragraphs -join "`n`n") + "`n", [Text.UTF8Encoding]::new($false))
        }
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

function Assert-CabinetPaths([string]$Path) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.ReadUInt32() -ne 0x4643534D) { throw 'Invalid CAB signature.' }
        $reader.BaseStream.Seek(16, [IO.SeekOrigin]::Begin) | Out-Null
        $fileOffset = $reader.ReadUInt32()
        $reader.BaseStream.Seek(28, [IO.SeekOrigin]::Begin) | Out-Null
        $count = $reader.ReadUInt16()
        $reader.BaseStream.Seek($fileOffset, [IO.SeekOrigin]::Begin) | Out-Null
        $paths = @{}
        for ($index = 0; $index -lt $count; $index++) {
            $reader.BaseStream.Seek(16, [IO.SeekOrigin]::Current) | Out-Null
            $name = [Collections.Generic.List[byte]]::new()
            while (($value = $reader.ReadByte()) -ne 0) { $name.Add($value) }
            $relative = [Text.Encoding]::UTF8.GetString($name.ToArray()).Replace('\', '/')
            Assert-RelativePath $relative
            if ($paths.ContainsKey($relative)) { throw "Duplicate cabinet path: $relative" }
            $paths[$relative] = $true
        }
    } finally { $reader.Dispose() }
}

function Assert-PeArchitecture([string]$Path, [string]$Architecture) {
    $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($Path))
    try {
        if ($reader.BaseStream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) { throw "Invalid Windows executable: $Path" }
        $reader.BaseStream.Seek(0x3C, [IO.SeekOrigin]::Begin) | Out-Null
        $offset = $reader.ReadUInt32()
        if ($offset -gt $reader.BaseStream.Length - 6) { throw "Invalid PE header: $Path" }
        $reader.BaseStream.Seek($offset, [IO.SeekOrigin]::Begin) | Out-Null
        if ($reader.ReadUInt32() -ne 0x00004550) { throw "Invalid PE signature: $Path" }
        $machine = $reader.ReadUInt16()
        $expected = if ($Architecture -eq 'x64') { 0x8664 } else { 0xAA64 }
        if ($machine -ne $expected) { throw "Fixed runtime architecture mismatch: $Path" }
    } finally { $reader.Dispose() }
}

function Expand-VerifiedZip([string]$Archive, [string]$Destination) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $root = [IO.Path]::GetFullPath($Destination) + [IO.Path]::DirectorySeparatorChar
        foreach ($entry in $zip.Entries) {
            Assert-RelativePath $entry.FullName.TrimEnd('/')
            $target = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $entry.FullName))
            if (-not $target.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Archive entry escapes resource directory: $($entry.FullName)"
            }
        }
    } finally { $zip.Dispose() }
    [System.IO.Compression.ZipFile]::ExtractToDirectory($Archive, $Destination)
}

function Assert-ResourceBundle([string]$Directory) {
    $manifest = Get-Content -LiteralPath (Join-Path $Directory 'manifest.json') -Raw | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or $manifest.architecture -notin @('x64', 'arm64')) {
        throw 'Unsupported desktop resource manifest.'
    }
    if ($manifest.architecture -ne $specification.architecture -or
        ($ExpectedArchitecture -and $manifest.architecture -ne $ExpectedArchitecture)) {
        throw 'Desktop resource architecture mismatch.'
    }
    if ($manifest.specification_sha256 -ine (Get-FileHash -LiteralPath $SpecificationPath -Algorithm SHA256).Hash) {
        throw 'Desktop resources do not match the reviewed acquisition specification; reacquire them.'
    }
    if ($manifest.speech.model -ne 'tiny.en' -or $manifest.speech.license -ne 'MIT') {
        throw 'The initial bundled speech model must be Whisper tiny.en with its MIT license.'
    }
    if ($manifest.webview2.version -ne $specification.webview2.version -or
        $manifest.webview2.archive_sha256 -ine $specification.webview2.sha256 -or
        $manifest.speech.sha256 -ine $specification.speech.sha256) { throw 'Desktop resource metadata differs from specification.' }
    Assert-Digest $manifest.webview2.archive_sha256
    Assert-Checksum (Join-Path $Directory 'speech/ggml-tiny.en.bin') $specification.speech.sha256
    if (-not (Test-Path -LiteralPath (Join-Path $Directory 'webview2/msedgewebview2.exe') -PathType Leaf)) {
        throw 'The fixed runtime is missing msedgewebview2.exe.'
    }
    Assert-PeArchitecture (Join-Path $Directory 'webview2/msedgewebview2.exe') $manifest.architecture
    foreach ($required in $specification.webview2.required_files) {
        Assert-RelativePath $required.path
        $path = Join-Path $Directory ('webview2/' + $required.path)
        Assert-Checksum $path $required.sha256
        if ((Get-Item -LiteralPath $path).VersionInfo.ProductVersion -ne $specification.webview2.version) {
            throw "Fixed runtime version mismatch: $path"
        }
    }
    foreach ($notice in $specification.notices) {
        Assert-Checksum (Join-Path $Directory ('notices/' + $notice.file)) $notice.output_sha256
    }
    $root = [IO.Path]::GetFullPath($Directory) + [IO.Path]::DirectorySeparatorChar
    $tracked = @{}
    foreach ($file in $manifest.files) {
        Assert-RelativePath $file.path
        $path = [IO.Path]::GetFullPath([IO.Path]::Combine($root, $file.path))
        if (-not $path.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { throw 'Manifest path escapes bundle.' }
        if ($tracked.ContainsKey($file.path)) { throw "Duplicate resource manifest entry: $($file.path)" }
        Assert-Checksum $path $file.sha256
        if ((Get-Item -LiteralPath $path).Length -ne $file.bytes) { throw "Inventory size mismatch: $($file.path)" }
        $tracked[$file.path] = $true
    }
    foreach ($required in @('speech/ggml-tiny.en.bin', 'webview2/msedgewebview2.exe', 'webview2/msedge.dll') +
        @($specification.notices | ForEach-Object { 'notices/' + $_.file })) {
        if (-not $tracked.ContainsKey($required)) { throw "Required resource is missing from inventory: $required" }
    }
    foreach ($file in Get-ChildItem -LiteralPath $Directory -Recurse -Force) {
        if ($file.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Resource links are not allowed: $($file.FullName)" }
        if ($file.PSIsContainer) { continue }
        $relative = $file.FullName.Substring($root.Length).Replace('\', '/')
        if ($relative -ne 'manifest.json' -and -not $tracked.ContainsKey($relative)) {
            throw "Untracked file in desktop resources: $relative"
        }
    }
    if ((Get-RuntimeInventoryDigest $manifest.files) -ine $specification.webview2.inventory_sha256) {
        throw 'Fixed runtime inventory differs from the reviewed signed archive.'
    }
}

function Get-RuntimeInventoryDigest($Files) {
    [string[]]$lines = @($Files | Where-Object { $_.path.StartsWith('webview2/') } | ForEach-Object {
        "$($_.path)`t$($_.bytes)`t$($_.sha256.ToLowerInvariant())"
    })
    [Array]::Sort($lines, [StringComparer]::Ordinal)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes(($lines -join "`n") + "`n")
        return ([BitConverter]::ToString($hasher.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally { $hasher.Dispose() }
}

$output = [IO.Path]::GetFullPath($OutputDirectory)
$specification = Get-Content -LiteralPath $SpecificationPath -Raw | ConvertFrom-Json
if ($ExpectedArchitecture -and $ExpectedArchitecture -ne $specification.architecture) {
    throw 'Requested architecture does not match the reviewed acquisition specification.'
}
if ($specification.schema_version -ne 1 -or $specification.architecture -notin @('x64', 'arm64')) {
    throw 'Acquisition specification must have schema_version 1 and architecture x64 or arm64.'
}
if ($specification.speech.model -ne 'tiny.en' -or $specification.speech.license -ne 'MIT') {
    throw 'Specify a reviewed GGML Whisper tiny.en model with its MIT license.'
}
if ($specification.webview2.version -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'An explicit fixed runtime version is required.' }
if ($specification.webview2.format -notin @('cab', 'zip')) { throw 'Fixed runtime archive format must be cab or zip.' }
foreach ($resource in @($specification.webview2, $specification.speech) + @($specification.notices)) {
    Assert-Digest $resource.sha256
    $url = [Uri]$resource.url
    if (-not $url.IsAbsoluteUri -or $url.Scheme -ne 'https') { throw 'Resource URLs must use HTTPS.' }
    if ($resource.bytes -le 0) { throw 'Every downloaded resource must specify its exact byte count.' }
}
Assert-Digest $specification.webview2.inventory_sha256
foreach ($notice in $specification.notices) {
    Assert-RelativePath $notice.file
    Assert-Digest $notice.output_sha256
}
if ($VerifyOnly) {
    Assert-ResourceBundle $output
    Write-Output "Verified bundled fixed WebView2 runtime, speech model, and notices: $output"
    return
}
if (Test-Path -LiteralPath $output) { throw "Destination exists; use -VerifyOnly or select a new destination: $output" }
$parent = Split-Path -Parent $output
if (-not (Test-Path -LiteralPath $parent -PathType Container)) { throw "Destination parent does not exist: $parent" }
if ($DownloadCacheDirectory -and -not (Test-Path -LiteralPath $DownloadCacheDirectory -PathType Container)) {
    throw "Download cache directory does not exist: $DownloadCacheDirectory"
}
if ($env:OS -ne 'Windows_NT') { throw 'Fixed runtime acquisition requires Windows for Microsoft signature verification and CAB extraction.' }
$staging = Join-Path $parent ('.desktop-resources-' + [Guid]::NewGuid().ToString('N'))
$downloads = Join-Path $staging 'downloads'
$bundle = Join-Path $staging 'bundle'
try {
    New-Item -ItemType Directory -Path $downloads, $bundle, (Join-Path $bundle 'webview2'),
        (Join-Path $bundle 'speech'), (Join-Path $bundle 'notices') | Out-Null
    $runtimeArchive = Join-Path $downloads ('runtime.' + $specification.webview2.format)
    Get-VerifiedResource $specification.webview2 $runtimeArchive
    $runtimeExtract = Join-Path $staging 'runtime'
    if ($specification.webview2.format -eq 'cab') {
        Assert-MicrosoftSignature $runtimeArchive $specification.webview2.authenticode_thumbprint
        Assert-CabinetPaths $runtimeArchive
        New-Item -ItemType Directory -Path $runtimeExtract | Out-Null
        & "$env:SystemRoot\System32\expand.exe" '-F:*' $runtimeArchive $runtimeExtract | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "Fixed runtime cabinet expansion failed: $LASTEXITCODE" }
    } else { Expand-VerifiedZip $runtimeArchive $runtimeExtract }
    $browserExecutables = @(Get-ChildItem -LiteralPath $runtimeExtract -Recurse -File -Filter 'msedgewebview2.exe')
    if ($browserExecutables.Count -ne 1) { throw 'Fixed runtime archive must contain exactly one msedgewebview2.exe.' }
    $runtimeSource = $browserExecutables[0].Directory.FullName
    Get-ChildItem -LiteralPath $runtimeSource -Force | Copy-Item -Destination (Join-Path $bundle 'webview2') -Recurse
    $version = $browserExecutables[0].VersionInfo.ProductVersion
    if ($version -ne $specification.webview2.version) { throw "Fixed runtime version mismatch: received $version" }
    foreach ($required in $specification.webview2.required_files) {
        Assert-MicrosoftSignature (Join-Path $bundle ('webview2/' + $required.path)) $specification.webview2.authenticode_thumbprint
    }

    Get-VerifiedResource $specification.speech (Join-Path $bundle 'speech/ggml-tiny.en.bin')
    foreach ($notice in $specification.notices) {
        Assert-RelativePath $notice.file
        Get-VerifiedNotice $notice (Join-Path $bundle ('notices/' + $notice.file)) $downloads
    }

    # Fixed runtimes on Windows 10 require read/execute access for their AppContainer.
    $permissionArguments = @((Join-Path $bundle 'webview2'), '/grant',
        '*S-1-15-2-1:(OI)(CI)(RX)', '*S-1-15-2-2:(OI)(CI)(RX)', '/T', '/Q')
    & "$env:SystemRoot\System32\icacls.exe" @permissionArguments
    if ($LASTEXITCODE -ne 0) { throw 'Could not grant WebView2 AppContainer runtime access.' }

    $root = [IO.Path]::GetFullPath($bundle) + [IO.Path]::DirectorySeparatorChar
    $files = @(Get-ChildItem -LiteralPath $bundle -Recurse -File -Force | Sort-Object FullName | ForEach-Object {
        [ordered]@{
            path = $_.FullName.Substring($root.Length).Replace('\', '/')
            sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
            bytes = $_.Length
        }
    })
    $manifest = [ordered]@{
        schema_version = 1
        architecture = $specification.architecture
        specification_sha256 = (Get-FileHash -LiteralPath $SpecificationPath -Algorithm SHA256).Hash.ToLowerInvariant()
        webview2 = [ordered]@{
            version = $specification.webview2.version
            url = $specification.webview2.url
            archive_sha256 = $specification.webview2.sha256.ToLowerInvariant()
            license = 'Microsoft WebView2 Runtime distribution terms'
            archive_bytes = $specification.webview2.bytes
            authenticode_thumbprint = $specification.webview2.authenticode_thumbprint
            verification = $specification.webview2.verification
        }
        speech = [ordered]@{
            model = 'tiny.en'
            file = 'speech/ggml-tiny.en.bin'
            url = $specification.speech.url
            sha256 = $specification.speech.sha256.ToLowerInvariant()
            license = 'MIT'
            bytes = $specification.speech.bytes
            revision = $specification.speech.revision
            verification = $specification.speech.verification
        }
        notices = $specification.notices
        files = $files
    }
    $json = $manifest | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText((Join-Path $bundle 'manifest.json'), $json, [Text.UTF8Encoding]::new($false))
    Assert-ResourceBundle $bundle
    Move-Item -LiteralPath $bundle -Destination $output
    Write-Output "Acquired verified desktop resources: $output"
} finally {
    if (Test-Path -LiteralPath $staging) { Remove-Item -LiteralPath $staging -Recurse -Force }
}
