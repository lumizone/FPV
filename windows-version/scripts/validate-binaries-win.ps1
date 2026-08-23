# Validate Windows runtime trees required by the NSIS bundle.
# Run from the repository root or through `npm run package:validate`.
[CmdletBinding()]
param(
    [switch]$ManifestOnly
)

$ErrorActionPreference = "Stop"
$Root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$Tauri = Join-Path $Root "src-tauri"
$Lock = Join-Path $Tauri "binaries-win.lock"
if (-not (Test-Path -LiteralPath $Lock -PathType Leaf)) { throw "Missing Windows binary manifest: $Lock" }
$Manifest = Get-Content -LiteralPath $Lock -Raw
if ($Manifest -match '(?im)^TARGET=(?!x86_64-pc-windows-msvc$)') { throw "Windows binary manifest has a non-Windows target" }
if ($Manifest -match '(?im)darwin|apple|\.dylib|ollama-darwin') { throw "Windows binary manifest contains Apple metadata" }
if ($Manifest -notmatch '(?im)^OLLAMA_URL=https://[^\r\n]*ollama-windows-amd64\.zip$') { throw "Windows manifest must pin the Ollama x64 archive URL" }
if ($Manifest -notmatch '(?im)^SD_CLI_URL=https://[^\r\n]*win-cpu-x64\.zip$') { throw "Windows manifest must pin the sd.cpp x64 archive URL" }

if ($ManifestOnly) {
    Write-Host "[binaries] Windows manifest valid: x86_64-pc-windows-msvc"
    exit 0
}

function Require-File([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Missing $Label`: $Path"
    }
}
function Require-Directory([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        throw "Missing $Label`: $Path"
    }
}

$Ollama = Join-Path $Tauri "ollama-runtime"
$Image = Join-Path $Tauri "image-runtime"
Require-Directory $Ollama "Ollama runtime"
Require-Directory $Image "sd.cpp runtime"
Require-File (Join-Path $Ollama "ollama.exe") "ollama.exe"
Require-File (Join-Path $Image "sd-cli.exe") "sd-cli.exe"
Require-Directory (Join-Path $Ollama "lib\ollama") "Ollama runner directory"

$dlls = @(Get-ChildItem -LiteralPath $Ollama -Recurse -File -Include *.dll -ErrorAction SilentlyContinue)
if ($dlls.Count -eq 0) {
    throw "Ollama runtime contains no DLLs"
}

$sdFiles = @(Get-ChildItem -LiteralPath $Image -Recurse -File)
if ($sdFiles.Count -lt 2) {
    throw "sd.cpp runtime is incomplete: expected sd-cli.exe and backend DLLs"
}

Write-Host "[binaries] valid: ollama-runtime and image-runtime"
