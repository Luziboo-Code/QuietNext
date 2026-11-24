param(
    [string]$Version
)
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$out = Join-Path $root 'dist'
$work = Join-Path $root 'target\setup'
$csc = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
if (!(Test-Path $csc)) { throw 'csc.exe (.NET Framework 4) is required' }
if (!$Version) {
    $Version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
}
$app = Join-Path $root 'target\release\QuietNext.exe'
if (!(Test-Path $app)) { throw "Build the release app first: $app" }
New-Item -ItemType Directory -Force $out, $work | Out-Null
Remove-Item (Join-Path $work '*') -Force -ErrorAction SilentlyContinue

$buildInfo = Join-Path $work 'BuildInfo.cs'
[System.IO.File]::WriteAllText($buildInfo, "static class BuildInfo { public const string Version = `"$Version`"; }`r`n", [System.Text.Encoding]::UTF8)

$refs = @('/r:System.dll', '/r:System.Core.dll', '/r:System.Windows.Forms.dll', '/r:System.Drawing.dll')
$icon = Join-Path $root 'assets\branding\app-icon.ico'

$uninstaller = Join-Path $work 'uninstall.exe'
& $csc /nologo /target:winexe /platform:anycpu /optimize+ "/win32icon:$icon" "/out:$uninstaller" $refs `
    (Join-Path $PSScriptRoot 'Uninstaller.cs') $buildInfo
if ($LASTEXITCODE -ne 0) { throw 'Uninstaller compilation failed' }

function Gzip([string]$source, [string]$dest) {
    $input = [System.IO.File]::OpenRead($source)
    try {
        $output = [System.IO.File]::Create($dest)
        try {
            $gz = New-Object System.IO.Compression.GZipStream($output, [System.IO.Compression.CompressionMode]::Compress)
            try { $input.CopyTo($gz) } finally { $gz.Dispose() }
        } finally { $output.Dispose() }
    } finally { $input.Dispose() }
}

$appGz = Join-Path $work 'app.gz'
$runtimeGz = Join-Path $work 'runtime.gz'
$runtime1Gz = Join-Path $work 'runtime1.gz'
$uninstallGz = Join-Path $work 'uninstall.gz'
Gzip $app $appGz
Gzip (Join-Path $env:WINDIR 'System32\vcruntime140.dll') $runtimeGz
Gzip (Join-Path $env:WINDIR 'System32\vcruntime140_1.dll') $runtime1Gz
Gzip $uninstaller $uninstallGz

$installer = Join-Path $out "QuietNext-$Version-windows-x64-setup.exe"
& $csc /nologo /target:winexe /platform:anycpu /optimize+ "/win32icon:$icon" "/out:$installer" $refs `
    "/resource:$appGz,QuietNext.app.gz" "/resource:$runtimeGz,QuietNext.runtime.gz" `
    "/resource:$runtime1Gz,QuietNext.runtime1.gz" "/resource:$uninstallGz,QuietNext.uninstall.gz" `
    (Join-Path $PSScriptRoot 'Installer.cs') $buildInfo
if ($LASTEXITCODE -ne 0) { throw 'Installer compilation failed' }

$hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
[System.IO.File]::WriteAllText("$installer.sha256", "$hash  $([System.IO.Path]::GetFileName($installer))`n", [System.Text.Encoding]::ASCII)
Write-Output $installer
Write-Output "$hash  $([System.IO.Path]::GetFileName($installer))"
