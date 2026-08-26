# fetch-binaries-win.ps1
# Prepares the Windows build inputs FPV needs to bundle self-contained,
# GPU-capable local AI with zero user setup (macOS-Metal parity):
#   1. The stable-diffusion.cpp CPU runtime (sd-cli.exe + backend DLLs) in
#      src-tauri/image-runtime/ - the always-works local image engine.
#      The ACCELERATED CUDA/ROCm build is NOT staged here: it is too large
#      for the NSIS installer and is fetched at runtime into app-data by
#      src-tauri/src/sidecars/gpu_runtime.rs.
#   2. The complete Ollama runtime (ollama.exe + lib/ollama GPU runners) in
#      src-tauri/ollama-runtime/.
# Both are bundled verbatim by tauri.windows.conf.json (bundle.resources);
# Ollama is launched in-place by sidecars/ollama.rs's spawn_bundled_windows,
# and sd-cli.exe is resolved by image/sdcpp.rs's sd_cli_path.
#
# Release pins live in src-tauri/binaries-win.lock. The macOS-only
# src-tauri/binaries.lock is never read by this script.
#
# Adapted from Local Waifu's win_version/local-waifu/scripts/
# fetch-binaries-win.ps1 (Ollama + stable-diffusion.cpp sections only - FPV
# does not need that project's ONNX Runtime / VC++ Redistributable /
# whisper.cpp / TTS placeholder sections).
#
# Usage (from repo root):
#   powershell -ExecutionPolicy Bypass -File scripts/fetch-binaries-win.ps1
#
# IMPORTANT: keep this file ASCII-only. GitHub's runner invokes it via
# Windows PowerShell 5.1, which reads the file in the system ANSI code
# page; a stray non-ASCII byte (e.g. a check mark) corrupts string
# parsing and breaks the whole script. Also avoid Join-Path with 3+
# positional segments - that overload does not exist on 5.1; use
# [IO.Path]::Combine instead.
#
# Requires: curl (built into Windows 10+), Expand-Archive (PowerShell 5+).

param(
    # Optional override for local testing; the Windows lock remains authoritative
    # in CI and is checked below.
    [string]$OllamaVersion = "",
    # makensis (NSIS) is 32-bit: it mmaps the collected UNCOMPRESSED
    # payload into a temp data file and dies once that file exceeds ~2 GB
    # ("error mmapping file ... out of range"). The full Ollama runtime
    # carries multiple CUDA major lines (each ~1-1.8 GB of ggml-cuda +
    # cuBLAS); we keep exactly ONE. The CPU runner is always present.
    #
    # $KeepCuda is the PREFERRED line. If that exact folder isn't in the
    # downloaded build (Ollama renames these across versions - 0.6.x had
    # cuda_v11/v12, 0.30.x ships cuda_v12/v13), the prune step below
    # auto-falls back to the lowest-versioned cuda_* present (best
    # NVIDIA-driver compatibility). So a stale value here can't strand us
    # with a CPU-only build.
    #   lower CUDA (v11/v12) -> wider driver compatibility (older drivers)
    #   higher CUDA (v13)    -> Ada/Hopper/Blackwell, needs a newer driver
    [string]$KeepCuda = "v12"
)

$ErrorActionPreference = "Stop"
$SrcTauri   = [IO.Path]::Combine($PSScriptRoot, "..", "src-tauri")
$LockFile   = Join-Path $SrcTauri "binaries-win.lock"
$RuntimeDir = [IO.Path]::Combine($SrcTauri, "ollama-runtime")
$TempDir    = Join-Path $env:TEMP "fpv-fetch-binaries-win"

if (-not (Test-Path -LiteralPath $LockFile -PathType Leaf)) {
    throw "Missing Windows binary manifest: $LockFile"
}
# Parse the lock as plain key/value text. Do NOT dot-source it: dot-sourcing
# is fragile here (on CRLF checkouts the value can carry a trailing \r, which
# breaks exact `-eq` and `-notmatch ...$` comparisons) and the macOS-only
# binaries.lock must never be sourced anyway.
$LockMap = @{}
foreach ($Line in Get-Content -LiteralPath $LockFile) {
    if ($Line -match '^([A-Za-z_][A-Za-z0-9_]*)=(.*)$') {
        $LockMap[$Matches[1]] = $Matches[2].Trim()
    }
}
$TARGET              = $LockMap['TARGET']
$OLLAMA_VERSION      = $LockMap['OLLAMA_VERSION']
$OLLAMA_URL          = $LockMap['OLLAMA_URL']
$OLLAMA_CHECKSUM_URL = $LockMap['OLLAMA_CHECKSUM_URL']
$SD_CLI_TAG          = $LockMap['SD_CLI_TAG']
$SD_CLI_ASSET        = $LockMap['SD_CLI_ASSET']
$SD_CLI_URL          = $LockMap['SD_CLI_URL']
$SD_CLI_RELEASE_URL  = $LockMap['SD_CLI_RELEASE_URL']
if ($TARGET -ne "x86_64-pc-windows-msvc") { throw "Windows manifest target must be x86_64-pc-windows-msvc" }
if ($OLLAMA_URL -notmatch 'ollama-windows-amd64\.zip$') { throw "Windows manifest Ollama URL is not the x64 archive" }
if ($SD_CLI_URL -notmatch 'win-cpu-x64\.zip$') { throw "Windows manifest sd.cpp URL is not the x64 archive" }
if ([string]::IsNullOrWhiteSpace($OllamaVersion)) { $OllamaVersion = $OLLAMA_VERSION }
if ($OllamaVersion -ne $OLLAMA_VERSION) { throw "OllamaVersion override must match binaries-win.lock" }

function Assert-Sha256([string]$Path, [string]$ExpectedHash, [string]$Label) {
    if ($ExpectedHash -notmatch '^[a-fA-F0-9]{64}$') {
        Write-Error "$Label upstream SHA256 is missing or malformed - refusing to continue."
        exit 1
    }
    $ActualHash = (Get-FileHash $Path -Algorithm SHA256).Hash
    if ($ActualHash -ne $ExpectedHash) {
        Write-Error "$Label SHA256 mismatch (got $ActualHash, expected $ExpectedHash) - refusing an unverified download."
        exit 1
    }
}

# -- 1. stable-diffusion.cpp CPU runtime (local image generation) -----
# Small CPU-only build bundled as a Windows resource so ImageProvider::Local
# (sd-cli.exe) renders out of the box on every machine, offline, with no
# download. The ACCELERATED CUDA/ROCm build is far too large to bundle
# (makensis is 32-bit and dies past ~2 GB uncompressed) and is fetched on
# first use into app-data by src-tauri/src/sidecars/gpu_runtime.rs; the model
# WEIGHTS are a separate runtime download (image_local_prewarm).
#
# Runs BEFORE the Ollama block below, because that block exits early once its
# runtime is already staged.
#
# The tag and asset name below were verified live against the GitHub release
# API on 2026-08-18: release master-769-cc73429 publishes
# sd-master-cc73429-bin-win-cpu-x64.zip alongside its cuda12 / rocm / vulkan
# siblings. Bump $SdTag and $SdAsset together, and keep SD_RELEASE_TAG /
# SD_RELEASE_BASE in sidecars/gpu_runtime.rs in lockstep - the bundled CPU
# build and the fetched GPU build should come from ONE upstream release.
$SdTag        = $SD_CLI_TAG
$SdAsset      = $SD_CLI_ASSET
$SdRuntimeDir = [IO.Path]::Combine($SrcTauri, "image-runtime")
$SdCli        = Join-Path $SdRuntimeDir "sd-cli.exe"
if (Test-Path $SdCli) {
    $ExistingSdDlls = @(Get-ChildItem -Path $SdRuntimeDir -Recurse -File -Filter "*.dll")
    if ($ExistingSdDlls.Count -eq 0) {
        Write-Error "Existing sd.cpp runtime has no backend DLLs; delete $SdRuntimeDir and retry."
        exit 1
    }
    Write-Host "[OK] stable-diffusion.cpp runtime already present: $SdRuntimeDir ($($ExistingSdDlls.Count) DLLs)"
    Write-Host "     Delete src-tauri/image-runtime to re-download."
} else {
    Write-Host "Downloading stable-diffusion.cpp $SdTag (CPU) for Windows x64..."
    New-Item -ItemType Directory -Force -Path $TempDir | Out-Null
    $SdZipUrl  = $SD_CLI_URL
    $SdZipPath = Join-Path $TempDir $SdAsset
    $SdExtract = Join-Path $TempDir "sd-extract"
    Write-Host "  URL: $SdZipUrl"
    curl.exe -L --fail --progress-bar -o $SdZipPath $SdZipUrl
    if ($LASTEXITCODE -ne 0) {
        Write-Error "sd.cpp download failed. Check the tag/asset name and network."
        exit 1
    }

    # sd.cpp publishes no sha256sum.txt for its releases (unlike Ollama
    # below). GitHub's release API DOES expose the immutable per-asset
    # digest, so require that upstream value rather than maintaining a
    # hand-copied hash in this script that nothing can re-derive.
    $SdReleaseUrl = $SD_CLI_RELEASE_URL
    # Anonymous api.github.com calls are capped at 60/hr PER IP, shared
    # across every GitHub-hosted runner behind the same NAT - exhausted by
    # unrelated traffic and NOT something a retry reliably fixes. Inside
    # Actions, GITHUB_TOKEN (passed through by the workflow) authenticates
    # at a far higher per-repo limit; fall back to anonymous locally.
    $SdHeaders = @{ "User-Agent" = "fpv-fetch-binaries" }
    if ($env:GITHUB_TOKEN) {
        $SdHeaders["Authorization"] = "Bearer $env:GITHUB_TOKEN"
    }
    try {
        $SdRelease = Invoke-RestMethod -Uri $SdReleaseUrl -Headers $SdHeaders
        $SdDigest = ($SdRelease.assets | Where-Object { $_.name -eq $SdAsset } | Select-Object -First 1).digest
    } catch {
        Write-Error "Could not retrieve the upstream sd.cpp release digest: $_"
        exit 1
    }
    $SdExpectedHash = $SdDigest -replace '^sha256:', ''
    Assert-Sha256 $SdZipPath $SdExpectedHash "stable-diffusion.cpp $SdAsset"

    Write-Host "  Extracting..."
    if (Test-Path $SdExtract) { Remove-Item -Recurse -Force $SdExtract }
    Expand-Archive -Path $SdZipPath -DestinationPath $SdExtract -Force
    $SdCliSrc = Get-ChildItem -Path $SdExtract -Recurse -Filter "sd-cli.exe" | Select-Object -First 1
    if (-not $SdCliSrc) {
        Write-Error "sd-cli.exe not found in the sd.cpp archive!"
        exit 1
    }
    # Copy the WHOLE directory (sd-cli.exe + its backend DLLs) so the binary
    # runs in place - the DLLs must sit beside the exe, which is why this is
    # bundled as a directory resource instead of a flat sidecar binary.
    if (Test-Path $SdRuntimeDir) { Remove-Item -Recurse -Force $SdRuntimeDir }
    New-Item -ItemType Directory -Force -Path $SdRuntimeDir | Out-Null
    Copy-Item -Path (Join-Path $SdCliSrc.DirectoryName "*") -Destination $SdRuntimeDir -Recurse -Force
    $SdBytes = (Get-ChildItem -Path $SdRuntimeDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
    $SdBackendDlls = @(Get-ChildItem -Path $SdRuntimeDir -Recurse -File -Filter "*.dll")
    if ($SdBackendDlls.Count -eq 0) {
        Write-Error "sd.cpp archive contains no backend DLLs; refusing an incomplete image runtime."
        exit 1
    }
    $SdMb = [math]::Round($SdBytes / 1MB, 0)
    Write-Host "[OK] stable-diffusion.cpp CPU runtime staged: $SdRuntimeDir ($SdMb MB; $($SdBackendDlls.Count) DLLs)"
}

# -- 2. Self-contained Ollama runtime --------------------------------
$RuntimeExe = Join-Path $RuntimeDir "ollama.exe"
$RuntimeLib = [IO.Path]::Combine($RuntimeDir, "lib", "ollama")
if ((Test-Path $RuntimeExe) -and (Test-Path $RuntimeLib)) {
    $ExistingRunnerFiles = @(Get-ChildItem -Path $RuntimeLib -Recurse -File -Include *.dll,*.so -ErrorAction SilentlyContinue)
    if ($ExistingRunnerFiles.Count -eq 0) {
        Write-Error "Existing Ollama runtime has no runner libraries; delete $RuntimeDir and retry."
        exit 1
    }
    Write-Host "[OK] Ollama runtime already present: $RuntimeDir ($($ExistingRunnerFiles.Count) runner files)"
    Write-Host "     Delete src-tauri/ollama-runtime to re-download."
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
    exit 0
}

Write-Host "Downloading Ollama $OllamaVersion for Windows x64..."
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

$ZipUrl = $OLLAMA_URL
$ZipPath = Join-Path $TempDir "ollama-windows-amd64.zip"
$ChecksumUrl = $OLLAMA_CHECKSUM_URL
$ChecksumPath = Join-Path $TempDir "ollama-sha256sum.txt"
$ExtractDir = Join-Path $TempDir "ollama-extract"

Write-Host "  URL: $ZipUrl"
curl.exe -L --fail --progress-bar -o $ZipPath $ZipUrl
if ($LASTEXITCODE -ne 0) {
    Write-Error "Download failed. Check the version tag and network."
    exit 1
}

# Ollama publishes this checksum file alongside the release assets. Resolve
# the named asset from it so changing $OllamaVersion cannot silently retain a
# hash for a different archive. Upstream's sha256sum.txt lines can carry an
# optional "./" filename prefix - match both forms.
curl.exe -L --fail --progress-bar -o $ChecksumPath $ChecksumUrl
if ($LASTEXITCODE -ne 0) {
    Write-Error "Ollama checksum download failed. Check the release assets and network."
    exit 1
}
$ChecksumLine = Get-Content $ChecksumPath | Where-Object { $_ -match '^[a-fA-F0-9]{64}\s+\*?(\./)?ollama-windows-amd64\.zip$' } | Select-Object -First 1
if (-not $ChecksumLine) {
    Write-Error "ollama-windows-amd64.zip checksum was not found in Ollama's upstream sha256sum.txt."
    exit 1
}
$OllamaExpectedHash = ($ChecksumLine -split '\s+')[0]
Assert-Sha256 $ZipPath $OllamaExpectedHash "Ollama $OllamaVersion archive"

Write-Host "  Extracting..."
if (Test-Path $ExtractDir) { Remove-Item -Recurse -Force $ExtractDir }
Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force

$OllamaExe = Get-ChildItem -Path $ExtractDir -Recurse -Filter "ollama.exe" | Select-Object -First 1
if (-not $OllamaExe) {
    Write-Error "ollama.exe not found in the archive!"
    exit 1
}
$OllamaDir = $OllamaExe.DirectoryName

# Copy the ENTIRE self-contained runtime (exe + lib/ollama + any DLLs)
# so ollama.exe and its GPU runner payload sit together exactly as
# Ollama ships them - that is what guarantees GPU-runner discovery at
# runtime (sidecars/ollama.rs runs it in-place with CWD = this folder).
if (Test-Path $RuntimeDir) { Remove-Item -Recurse -Force $RuntimeDir }
New-Item -ItemType Directory -Force -Path $RuntimeDir | Out-Null
Copy-Item -Path (Join-Path $OllamaDir "*") -Destination $RuntimeDir -Recurse -Force

# Prune CUDA runner line(s) we are NOT shipping. Each lib/ollama/
# cuda_vXX folder holds a ~1-1.8 GB ggml-cuda.dll + cuBLAS; carrying
# more than one blows past the NSIS 2 GB installer ceiling.
#
# Robust against Ollama renaming the CUDA runner dirs across versions
# (0.6.x shipped cuda_v11/cuda_v12; newer builds moved to cuda_v12/
# cuda_v13). We KEEP exactly one:
#   - the one named in $KeepCuda if that folder actually exists, else
#   - the LOWEST-versioned cuda_* present (best NVIDIA-driver
#     compatibility - older CUDA needs an older minimum driver).
# Hard-coding a single name silently prunes ALL runners when the names
# change, shipping a CPU-only build with no GPU acceleration.
# Auto-selection prevents that.
$LibOllama = [IO.Path]::Combine($RuntimeDir, "lib", "ollama")
$CudaDirs = @(Get-ChildItem -Path $LibOllama -Directory -Filter "cuda_*" -ErrorAction SilentlyContinue)
if ($CudaDirs.Count -gt 0) {
    $Preferred = $KeepCuda.Split(",") | ForEach-Object { "cuda_$($_.Trim())" }
    $KeepDir = $CudaDirs | Where-Object { $Preferred -contains $_.Name } | Select-Object -First 1
    if (-not $KeepDir) {
        # Requested line absent in this Ollama version - fall back to the
        # lowest-versioned CUDA runner present (most compatible).
        # Sort NUMERICALLY on the version suffix, not alphabetically:
        # `Sort-Object Name` on strings orders "cuda_v10" before "cuda_v9"
        # (lexicographic "1" < "9"), so if Ollama ever ships a single-digit
        # major alongside a double-digit one, the alphabetic sort would
        # keep the NEWER, less-compatible runner instead of the oldest one
        # this fallback exists to prefer. Today's lineup (v12/v13) is all
        # double-digit, so both sorts happen to agree - this only bites a
        # future Ollama release, silently, unless fixed now.
        $KeepDir = $CudaDirs | Sort-Object { [int]($_.Name -replace '^cuda_v', '') } | Select-Object -First 1
        Write-Host "  NOTE: requested CUDA line ($KeepCuda) not in this build; keeping $($KeepDir.Name) instead."
    }
    foreach ($d in $CudaDirs) {
        if ($d.Name -eq $KeepDir.Name) {
            Write-Host "  Keeping GPU runner: $($d.Name)"
        } else {
            Write-Host "  Pruning GPU runner not shipped (NSIS size limit): $($d.Name)"
            Remove-Item -Recurse -Force $d.FullName
        }
    }
} else {
    Write-Host "  NOTE: no cuda_* runner dirs found under lib/ollama - CPU/other backends only."
}

# Verify the result is complete.
if (-not (Test-Path $RuntimeExe)) {
    Write-Error "ollama.exe missing in $RuntimeDir after copy."
    exit 1
}
if (-not (Test-Path $RuntimeLib)) {
    Write-Error "lib/ollama missing in the Ollama runtime; refusing a CPU-only/incomplete bundle."
    exit 1
}
$RunnerFiles = @(Get-ChildItem -Path $RuntimeLib -Recurse -File -Include *.dll,*.so -ErrorAction SilentlyContinue)
if ($RunnerFiles.Count -eq 0) {
    Write-Error "lib/ollama contains no runner libraries; refusing an incomplete bundle."
    exit 1
}
$Bytes = (Get-ChildItem -Path $RuntimeDir -Recurse -File | Measure-Object -Property Length -Sum).Sum
$Mb = [math]::Round($Bytes / 1MB, 0)
Write-Host "[OK] Bundled self-contained Ollama runtime: $RuntimeDir ($Mb MB; $($RunnerFiles.Count) runner files)"

Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Done. The installer will ship a GPU-capable Ollama plus a CPU sd-cli.exe"
Write-Host "      - user installs nothing. The accelerated image build is fetched later."
Write-Host "Next: npm run tauri -- build --bundles nsis"
