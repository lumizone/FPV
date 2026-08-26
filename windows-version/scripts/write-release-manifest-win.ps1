# Write a release manifest for a Windows installer artifact.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Artifact,
    [string]$Output
)

$ErrorActionPreference = "Stop"
$Root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
# The workflow passes ABSOLUTE paths (Get-ChildItem.FullName); Join-Path
# would blindly append them to the current directory ("D:\a\...\D:\a\..."),
# which then makes GetFullPath throw "The given path's format is not
# supported". Resolve absolute paths as-is, join only relative ones.
if ([IO.Path]::IsPathRooted($Artifact)) {
    $Artifact = [IO.Path]::GetFullPath($Artifact)
} else {
    $Artifact = [IO.Path]::GetFullPath((Join-Path (Get-Location) $Artifact))
}
if (-not (Test-Path -LiteralPath $Artifact -PathType Leaf)) {
    throw "Artifact does not exist: $Artifact"
}
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = "$Artifact.release-manifest.json"
} elseif ([IO.Path]::IsPathRooted($Output)) {
    $Output = [IO.Path]::GetFullPath($Output)
} else {
    $Output = [IO.Path]::GetFullPath((Join-Path (Get-Location) $Output))
}
$Cargo = Join-Path $Root "src-tauri\Cargo.toml"
$Version = ((Select-String -Path $Cargo -Pattern '^version = "([^"]+)"' | Select-Object -First 1).Matches.Groups[1].Value)
if ([string]::IsNullOrWhiteSpace($Version)) { throw "Could not read Cargo package version" }
$Hash = (Get-FileHash -LiteralPath $Artifact -Algorithm SHA256).Hash.ToLowerInvariant()
$Commit = (git -C $Root rev-parse HEAD 2>$null)
if ([string]::IsNullOrWhiteSpace($Commit)) { $Commit = "unknown" }
$Manifest = [ordered]@{
    product = "FPV"
    version = $Version
    target = "x86_64-pc-windows-msvc"
    artifact = [IO.Path]::GetFileName($Artifact)
    artifact_sha256 = $Hash
    artifact_size_bytes = (Get-Item -LiteralPath $Artifact).Length
    commit = $Commit.Trim()
    signed = $false
}
$Manifest | ConvertTo-Json | Set-Content -LiteralPath $Output -Encoding UTF8
Write-Host "[manifest] wrote $Output"
