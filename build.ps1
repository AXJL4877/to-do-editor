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
    # Rebuild the generated directory from scratch so stale local settings and
    # nested copies cannot survive between package runs.
    if (Test-Path -LiteralPath $taskDestination) {
        [IO.Directory]::Delete($taskDestination, $true)
    }
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
    # Extensions and the standard-library worker SDK are application data, not
    # Rust resources. Never copy tests or machine-local AI settings.
    $taskExtensions = Join-Path $taskRoot 'extensions'
    if (Test-Path -LiteralPath $taskExtensions) {
        $taskPackagedExtensions = Join-Path $taskDestination 'extensions'
        New-Item -ItemType Directory -Path $taskPackagedExtensions -Force | Out-Null
        Get-ChildItem -LiteralPath $taskExtensions -File -Recurse |
            Where-Object { $_.FullName -notmatch '[\\/]tests[\\/]' -and $_.Name -notlike '*.pyc' } |
            ForEach-Object {
                $taskRelative = $_.FullName.Substring($taskExtensions.Length).TrimStart([char]92, [char]47)
                $taskTarget = Join-Path $taskPackagedExtensions $taskRelative
                New-Item -ItemType Directory -Path (Split-Path $taskTarget) -Force | Out-Null
                Copy-Item -LiteralPath $_.FullName -Destination $taskTarget -Force
            }
    }
    $taskPython = Join-Path $taskRoot 'dependencies/python'
    if (Test-Path -LiteralPath $taskPython) {
        Copy-Item -LiteralPath $taskPython -Destination (Join-Path $taskDestination 'dependencies/python') -Recurse -Force
    } else {
        Write-Warning 'Bundled Python is missing; run scripts/fetch_python.ps1 before shipping.'
    }
    $taskExample = Join-Path $taskRoot 'config/ai.example.json'
    if (Test-Path -LiteralPath $taskExample) {
        New-Item -ItemType Directory -Path (Join-Path $taskDestination 'config') -Force | Out-Null
        Copy-Item -LiteralPath $taskExample -Destination (Join-Path $taskDestination 'config/ai.example.json') -Force
    }
    # A package must never contain credentials or copied local settings.
    $taskForbidden = '密匙|MISHI|api_key\s*=|(?<![A-Za-z])sk-[A-Za-z0-9]{8,}'
    $taskTextExtensions = @('.rs', '.json', '.toml', '.md', '.txt', '.py', '.ps1', '.bat', '.sh', '.lock')
    $taskHits = Get-ChildItem -LiteralPath $taskDestination -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $taskTextExtensions -contains $_.Extension.ToLowerInvariant() } |
        Select-String -Pattern $taskForbidden -SimpleMatch:$false -ErrorAction SilentlyContinue
    if ($taskHits) { throw "Possible credential text found in package: $($taskHits[0].Path)" }
    Write-Host "Package ready: $taskDestination"
} finally { Pop-Location }
