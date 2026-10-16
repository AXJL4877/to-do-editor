# Build the native preview when needed, then run the Rust desktop app.
$ErrorActionPreference = 'Stop'
$taskRoot = $PSScriptRoot
$taskNative = Join-Path $taskRoot 'native/build/bin/preview_host.exe'
$taskLatest = Get-ChildItem -LiteralPath (Join-Path $taskRoot 'native/src') -File -Recurse | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
if (-not (Test-Path -LiteralPath $taskNative) -or $taskLatest.LastWriteTimeUtc -gt (Get-Item -LiteralPath $taskNative).LastWriteTimeUtc) {
    & (Join-Path $taskRoot 'native/build.cmd')
    if ($LASTEXITCODE -ne 0) { throw 'Native preview build failed' }
}
$env:TODO_REPO_ROOT = $taskRoot
Push-Location $taskRoot
try { cargo run -p todo_app @args; $taskExit = $LASTEXITCODE } finally { Pop-Location }
exit $taskExit
