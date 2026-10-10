# Offline validator regression tests using a synthetic model/license fixture.
# Official resource hashes remain pinned in desktop-resources.json; these tests
# exercise validation logic without downloading a model or browser runtime.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$scriptPath = Join-Path $PSScriptRoot 'acquire-desktop-resources.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
foreach ($definition in $ast.FindAll({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst]}, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
function Assert-Rejected([scriptblock]$operation, [string]$message) {
    try { & $operation } catch {
        if ($_.Exception.Message -notlike "*$message*") { throw }
        Write-Output "PASS: rejects $message"
        return
    }
    throw "Expected rejection: $message"
}
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('system-browser-resources-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture, (Join-Path $fixture 'speech'), (Join-Path $fixture 'notices') | Out-Null
$SpecificationPath = Join-Path ([IO.Path]::GetTempPath()) ('resource-spec-' + [Guid]::NewGuid().ToString('N') + '.json')
try {
    $model = Join-Path $fixture 'speech/ggml-tiny.en.bin'
    $notice = Join-Path $fixture 'notices/Whisper.txt'
    [IO.File]::WriteAllText($model, 'synthetic speech fixture')
    [IO.File]::WriteAllText($notice, 'synthetic license fixture')
    $specification = [pscustomobject]@{
        schema_version = 1; architecture = 'x64'
        browser = @{windows_runtime='evergreen';macos_runtime='wkwebview'}
        speech = @{model='tiny.en';license='MIT';sha256=(Get-FileHash $model).Hash;bytes=(Get-Item $model).Length}
        notices = @(@{file='Whisper.txt';output_sha256=(Get-FileHash $notice).Hash;output_bytes=(Get-Item $notice).Length})
    }
    $specification | ConvertTo-Json -Depth 10 | Set-Content $SpecificationPath
    $ExpectedArchitecture = 'x64'
    $files = @(Get-ChildItem $fixture -File -Recurse | ForEach-Object {
        @{path=$_.FullName.Substring($fixture.Length+1).Replace('\','/');sha256=(Get-FileHash $_.FullName).Hash;bytes=$_.Length}
    })
    $manifest = @{schema_version=1;architecture='x64';specification_sha256=(Get-FileHash $SpecificationPath).Hash;
        browser=$specification.browser;speech=$specification.speech;files=$files}
    $manifestPath = Join-Path $fixture 'manifest.json'
    $manifest | ConvertTo-Json -Depth 10 | Set-Content $manifestPath
    $originalManifest = [IO.File]::ReadAllText($manifestPath)
    Assert-ResourceBundle $fixture
    Write-Output 'PASS: speech-only bundle with system browser metadata'

    [IO.File]::WriteAllText($model, 'tampered model')
    Assert-Rejected { Assert-ResourceBundle $fixture } 'SHA-256 mismatch'
    [IO.File]::WriteAllText($model, 'synthetic speech fixture')
    $malicious = $originalManifest | ConvertFrom-Json
    $malicious.files[0].sha256 = '0' * 64
    $malicious | ConvertTo-Json -Depth 10 | Set-Content $manifestPath
    Assert-Rejected { Assert-ResourceBundle $fixture } 'Inventory differs'
    [IO.File]::WriteAllText($manifestPath, $originalManifest)

    New-Item -ItemType Directory -Path (Join-Path $fixture 'webview2') | Out-Null
    Assert-Rejected { Assert-ResourceBundle $fixture } 'Private browser runtimes'
    Remove-Item (Join-Path $fixture 'webview2')
    [IO.File]::WriteAllText((Join-Path $fixture 'extra.bin'), 'unapproved')
    Assert-Rejected { Assert-ResourceBundle $fixture } 'Untracked file'
    Remove-Item (Join-Path $fixture 'extra.bin')
    $malicious = $originalManifest | ConvertFrom-Json
    $malicious.files = @($malicious.files | Where-Object {$_.path -ne 'notices/Whisper.txt'})
    $malicious | ConvertTo-Json -Depth 10 | Set-Content $manifestPath
    Assert-Rejected { Assert-ResourceBundle $fixture } 'Required resource is missing'
    [IO.File]::WriteAllText($manifestPath, $originalManifest)
    $ExpectedArchitecture = 'arm64'
    Assert-Rejected { Assert-ResourceBundle $fixture } 'architecture/schema mismatch'
    $ExpectedArchitecture = 'x64'
    $malicious = $originalManifest | ConvertFrom-Json
    $malicious.specification_sha256 = '0' * 64
    $malicious | ConvertTo-Json -Depth 10 | Set-Content $manifestPath
    Assert-Rejected { Assert-ResourceBundle $fixture } 'reviewed acquisition specification'
    [IO.File]::WriteAllText($manifestPath, $originalManifest)

    foreach ($path in @('../escape','/absolute','C:/absolute','file:stream','folder/../escape','folder\escape')) {
        Assert-Rejected { Assert-RelativePath $path } 'path escapes bundle'
    }
    $DownloadCacheDirectory = $null
    Assert-Rejected { Get-VerifiedResource @{url='http://example.invalid';sha256=('0' * 64)} (Join-Path $fixture 'unused') } 'HTTPS'
    Assert-ResourceBundle $fixture
    Write-Output 'PASS: checksums, reviewed inventory, architecture, stale caches, paths and no bundled browser'
} finally {
    Remove-Item -LiteralPath $fixture -Recurse -Force
    Remove-Item -LiteralPath $SpecificationPath -Force
}
