# Compile and execute the real Inno dependency functions with offline doubles.
# No network, registry writes, WebView2 installation, or KnightCode installation.
[CmdletBinding()]
param([string]$InnoCompiler)
$ErrorActionPreference = 'Stop'
if (-not $InnoCompiler) {
    $InnoCompiler = @("${env:ProgramFiles(x86)}/Inno Setup 6/ISCC.exe", "$env:LOCALAPPDATA/Programs/Inno Setup 6/ISCC.exe") |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}
if (-not $InnoCompiler) { throw 'Inno Setup 6 is required for the installer regression tests.' }
$installer = [IO.File]::ReadAllText((Join-Path $PSScriptRoot '../crates/zed/resources/windows/zed.iss'))
$start = $installer.IndexOf('function IsWebView2Version(')
$end = $installer.IndexOf('procedure CurStepChanged(', $start)
if ($start -lt 0 -or $end -lt 0) { throw 'Installer dependency functions not found.' }
$functions = $installer.Substring($start, $end - $start)
$functions = $functions.Replace('RegQueryStringValue(', 'FixtureQuery(').Replace('IsWin64', 'True')
$functions = $functions.Replace('DownloadTemporaryFile(', 'FixtureDownload(').Replace('Exec(', 'FixtureExec(')
$fixture = Join-Path ([IO.Path]::GetTempPath()) ('knightcode-evergreen-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
$script = @'
[Setup]
AppName=KnightCode Evergreen dependency tests
AppVersion=0.0.0
DefaultDirName={tmp}\not-installed
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
OutputDir=.
OutputBaseFilename=evergreen-tests
[Code]
var
  Machine32, Machine64, User32, User64: string;
  Downloads, SignatureChecks, BootstrapperRuns: Integer;
  Offline, Trusted, Installs: Boolean;
function FixtureQuery(RootKey: Integer; const Key, ValueName: string; var Value: string): Boolean;
begin
  if (Pos('{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}', Key) = 0) or (ValueName <> 'pv') then
    RaiseException('Unexpected registry lookup');
  Value := '';
  if RootKey = HKLM32 then Value := Machine32;
  if RootKey = HKLM64 then Value := Machine64;
  if RootKey = HKCU32 then Value := User32;
  if RootKey = HKCU64 then Value := User64;
  Result := Value <> '';
end;
function FixtureDownload(const Url, Name, Hash: string; const Progress: TOnDownloadProgress): Int64;
begin
  Downloads := Downloads + 1;
  if Url <> 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' then RaiseException('Wrong bootstrapper URL');
  if Offline then RaiseException('Simulated offline download');
  Result := 1;
end;
function FixtureExec(const Name, Parameters, Directory: string; Show: Integer; Wait: TExecWait; var Code: Integer): Boolean;
begin
  Result := True;
  if Pos('powershell.exe', Name) > 0 then
  begin
    SignatureChecks := SignatureChecks + 1;
    Code := 1;
    if Trusted then Code := 0;
  end
  else
  begin
    if Parameters <> '/silent /install' then RaiseException('Wrong bootstrapper arguments');
    BootstrapperRuns := BootstrapperRuns + 1;
    Code := 5;
    if Installs then
    begin
      Machine32 := '155.0.4283.45';
      Code := 0;
    end;
  end;
end;
procedure ResetFixture();
begin
  Machine32 := ''; Machine64 := ''; User32 := ''; User64 := '';
  Downloads := 0; SignatureChecks := 0; BootstrapperRuns := 0;
  Offline := False; Trusted := True; Installs := True;
end;
procedure Check(Value: Boolean; const Message: string);
begin
  if not Value then RaiseException('FAIL: ' + Message);
  Log('PASS: ' + Message);
end;
'@
$tests = @'
function InitializeSetup(): Boolean;
begin
  Check(IsWebView2Version('155.0.4283.45'), 'usable version');
  Check(IsWebView2Version('0.0.0.1'), 'any version greater than zero');
  Check(not IsWebView2Version(''), 'empty version rejected');
  Check(not IsWebView2Version('0.0.0.0'), 'zero version rejected');
  Check(not IsWebView2Version('0.00.00.00'), 'noncanonical zero version rejected');
  Check(not IsWebView2Version('garbage'), 'malformed version rejected');
  Check(not IsWebView2Version('.1.2.3'), 'leading dot rejected');
  Check(not IsWebView2Version('1..2.3'), 'empty component rejected');
  Check(not IsWebView2Version('1.2.3.'), 'trailing dot rejected');
  Check(not IsWebView2Version('1.2.3'), 'missing component rejected');
  Check(not IsWebView2Version('1.2.3.4.5'), 'extra component rejected');
  ResetFixture(); Machine32 := '155.0.4283.45'; EnsureSystemWebView2();
  Check((Downloads = 0) and (BootstrapperRuns = 0), 'HKLM32 skips download');
  ResetFixture(); User32 := '155.0.4283.45'; EnsureSystemWebView2();
  Check(Downloads = 0, 'HKCU32 skips download');
  ResetFixture(); User64 := '155.0.4283.45'; EnsureSystemWebView2();
  Check(Downloads = 0, 'HKCU64 skips download');
  ResetFixture(); Machine64 := '155.0.4283.45'; EnsureSystemWebView2();
  Check(Downloads = 0, 'HKLM64 fallback skips download');
  ResetFixture(); Machine32 := '0.0.0.0'; EnsureSystemWebView2();
  Check((Downloads = 1) and (SignatureChecks = 1) and (BootstrapperRuns = 1) and HasSystemWebView2(), 'missing runtime installs silently after signature check');
  ResetFixture(); Offline := True; EnsureSystemWebView2();
  Check((Downloads = 1) and (BootstrapperRuns = 0), 'offline failure is nonfatal');
  ResetFixture(); Trusted := False; EnsureSystemWebView2();
  Check((SignatureChecks = 1) and (BootstrapperRuns = 0), 'untrusted download is never executed');
  ResetFixture(); Installs := False; EnsureSystemWebView2();
  Check((BootstrapperRuns = 1) and not HasSystemWebView2(), 'failed runtime installation is nonfatal');
  Log('EVERGREEN INSTALLER REGRESSION TESTS PASSED');
  Result := False; // Stop before installing anything; expected setup exit code is nonzero.
end;
'@
$source = Join-Path $fixture 'evergreen-tests.iss'
[IO.File]::WriteAllText($source, $script + "`r`n" + $functions + "`r`n" + $tests)
& $InnoCompiler /Qp $source
if ($LASTEXITCODE -ne 0) { throw 'Inno Evergreen regression fixture did not compile.' }
$log = Join-Path $fixture 'tests.log'
$process = Start-Process -FilePath (Join-Path $fixture 'evergreen-tests.exe') `
    -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', ('/LOG="' + $log + '"')) -Wait -PassThru
if (-not (Test-Path $log) -or -not ([IO.File]::ReadAllText($log).Contains('EVERGREEN INSTALLER REGRESSION TESTS PASSED'))) {
    throw "Inno dependency tests failed; preserved fixture/log: $fixture"
}
Get-Content $log | Where-Object { $_ -match 'PASS:|REGRESSION TESTS PASSED' } | Write-Output
Write-Output "Preserved test fixture/log (no app installed): $fixture"
