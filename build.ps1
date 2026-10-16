param([ValidateSet('debug', 'release')][string]$Configuration = 'release')
$ErrorActionPreference = 'Stop'
$taskRoot = $PSScriptRoot
$taskTools = Join-Path $taskRoot 'dependencies/ffmpeg'
$taskFfmpeg = if ($env:TODO_FFMPEG) { $env:TODO_FFMPEG } else { Join-Path $taskTools 'bin/ffmpeg.exe' }
$taskFfprobe = if ($env:TODO_FFPROBE) { $env:TODO_FFPROBE } else { Join-Path $taskTools 'bin/ffprobe.exe' }
foreach ($taskTool in @($taskFfmpeg, $taskFfprobe)) {
    if (-not (Test-Path -LiteralPath $taskTool -PathType Leaf)) { throw "Missing packaged media tool: $taskTool. See dependencies/README.md." }
}
$taskNative = Join-Path $taskRoot 'native/build/bin/preview_host.exe'
$taskLatest = Get-ChildItem -LiteralPath (Join-Path $taskRoot 'native/src') -File -Recurse | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if (-not (Test-Path -LiteralPath $taskNative) -or $taskLatest.LastWriteTimeUtc -gt (Get-Item -LiteralPath $taskNative).LastWriteTimeUtc) {
    & (Join-Path $taskRoot 'native/build.cmd')
    if ($LASTEXITCODE -ne 0) { throw 'Native preview build failed' }
}
Push-Location $taskRoot
try {
    # Keep packaging independent of a running development executable on Windows.
    $taskBuildDirectory = Join-Path $taskRoot 'target/package'
    $taskCargoArgs = @('build', '-p', 'todo_app', '--target-dir', $taskBuildDirectory)
    if ($Configuration -eq 'release') { $taskCargoArgs += '--release' }
    & cargo @taskCargoArgs
    if ($LASTEXITCODE -ne 0) { throw 'Rust application build failed' }
    $taskDestination = [IO.Path]::GetFullPath((Join-Path $taskRoot 'dist/to_do'))
    if (-not $taskDestination.StartsWith($taskRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Package path escaped workspace' }
    $taskPackagedTools = Join-Path $taskDestination 'dependencies/ffmpeg/bin'
    New-Item -ItemType Directory -Path $taskPackagedTools -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $taskBuildDirectory "$Configuration/todo_app.exe") -Destination (Join-Path $taskDestination 'todo_app.exe') -Force
    Copy-Item -LiteralPath $taskNative -Destination (Join-Path $taskDestination 'preview_host.exe') -Force
    Copy-Item -LiteralPath $taskFfmpeg -Destination (Join-Path $taskPackagedTools 'ffmpeg.exe') -Force
    Copy-Item -LiteralPath $taskFfprobe -Destination (Join-Path $taskPackagedTools 'ffprobe.exe') -Force
    foreach ($taskNotice in @('LICENSE', 'README.txt')) {
        $taskNoticePath = Join-Path $taskTools $taskNotice
        if (Test-Path -LiteralPath $taskNoticePath) { Copy-Item -LiteralPath $taskNoticePath -Destination (Join-Path $taskDestination "dependencies/ffmpeg/$taskNotice") -Force }
    }
    $taskFonts = Join-Path $taskRoot 'dependencies/fonts'
    if (Test-Path -LiteralPath $taskFonts) {
        # Text styles name font files; ship them (and their notices) beside the app.
        $taskPackagedFonts = Join-Path $taskDestination 'dependencies/fonts'
        New-Item -ItemType Directory -Path $taskPackagedFonts -Force | Out-Null
        Get-ChildItem -LiteralPath $taskFonts -File | Copy-Item -Destination $taskPackagedFonts -Force
    }
    Write-Host "Package ready: $taskDestination"
} finally { Pop-Location }
