param(
    [string]$PythonVersion = "3.13.11",
    [string]$PythonUrl = "https://www.python.org/ftp/python/3.13.11/python-3.13.11-embed-amd64.zip",
    [string]$Root = (Resolve-Path (Join-Path $PSScriptRoot ".."))
)

$ErrorActionPreference = "Stop"
$target = Join-Path $Root "dependencies/python"
$archive = Join-Path $Root "dependencies/python-$PythonVersion-embed-amd64.zip"
$lock = Join-Path $Root "dependencies/python.lock"

New-Item -ItemType Directory -Force -Path (Split-Path $archive), $target | Out-Null
Invoke-WebRequest -Uri $PythonUrl -OutFile $archive
Expand-Archive -LiteralPath $archive -DestinationPath $target -Force
$pth = Get-ChildItem -LiteralPath $target -Filter "python*._pth" | Select-Object -First 1
if (-not $pth) { $pth = Get-ChildItem -LiteralPath $target -Filter "python*.pth" | Select-Object -First 1 }
if (-not $pth) { throw "embedded Python ._pth file was not found" }
$text = Get-Content -LiteralPath $pth.FullName -Raw
if ($text -notmatch "(?m)^import site\s*$") { Add-Content -LiteralPath $pth.FullName -Value "`nimport site`n" }
$python = Join-Path $target "python.exe"
if (-not (Test-Path $python)) { throw "python.exe was not extracted" }
$site = Join-Path $target "Lib/site-packages"
New-Item -ItemType Directory -Force -Path $site | Out-Null
& $python -m pip --version | Out-Null
if ($LASTEXITCODE -eq 0) {
    & $python -m pip install --no-deps --require-hashes -r $lock --target $site
} else {
    # The embeddable distribution may not ship pip. Validate and fetch the
    # exact wheels with the developer Python, then extract them with the
    # bundled interpreter so the shipped runtime remains self-contained.
    $wheelDir = Join-Path $target ".wheels"
    New-Item -ItemType Directory -Force -Path $wheelDir | Out-Null
    $systemPython = (Get-Command python.exe -ErrorAction Stop).Source
    & $systemPython -m pip download --require-hashes --no-deps --only-binary=:all: `
        --platform win_amd64 --python-version 313 --implementation cp --abi cp313 `
        -r $lock -d $wheelDir
    $extract = @'
import pathlib, sys, zipfile
root = pathlib.Path(sys.argv[1]); target = pathlib.Path(sys.argv[2]); target.mkdir(parents=True, exist_ok=True)
for wheel in root.glob("*.whl"):
    with zipfile.ZipFile(wheel) as archive: archive.extractall(target)
'@
    & $python -c $extract $wheelDir $site
}
if ($LASTEXITCODE -ne 0) { throw "Python dependency installation failed" }
Remove-Item -LiteralPath (Join-Path $target ".wheels") -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "Installed embedded Python $PythonVersion at $target"
