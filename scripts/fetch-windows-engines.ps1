<#
.SYNOPSIS
  Fills src-tauri/resources/engines with the Windows binaries an installer has
  to carry, fetching each from its upstream release.

.DESCRIPTION
  This is the build-host half of the engine tree. Weights are not staged here:
  they are ~2.3 GB against a 2 GB cap on a release asset, so the app downloads
  them into app data on first launch (see src-tauri/src/infrastructure/models.rs).

  The one model that does ship is the Piper voice, because it is 65 MB and,
  in the NavGurukul case, not publicly downloadable. -VoiceUrl supplies it in
  CI from a secret; without one, the public en_US voice is staged and Ella
  falls back to it exactly as she does on a development machine.
#>
param(
  [string]$Destination = (Join-Path (Split-Path -Parent $PSScriptRoot) "src-tauri\resources\engines"),
  # A private URL for en_IN-navgurukul-medium.onnx. Its .onnx.json sidecar is
  # expected at the same URL with .json appended — Piper reads the sample rate
  # and phoneme map from it and will not synthesize without it.
  [string]$VoiceUrl = $env:ELLA_PIPER_VOICE_URL
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # Invoke-WebRequest's bar is very slow in CI

function Get-ReleaseAsset {
  param(
    [string]$Repo,
    [string]$Pattern,
    # Pin a specific release tag for a reproducible build. Without one the
    # newest release that actually carries a matching asset wins.
    [string]$Tag
  )

  $headers = @{ "User-Agent" = "ella-build" }
  if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $env:GITHUB_TOKEN" }

  if ($Tag) {
    $releases = @(Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases/tags/$Tag")
  } else {
    # Not /releases/latest: llama.cpp marks every binary build a prerelease,
    # so "latest" is a source-only tag carrying no Windows assets at all.
    # Walk the list newest-first and take the first release that has one.
    $releases = @(Invoke-RestMethod -Headers $headers -Uri "https://api.github.com/repos/$Repo/releases?per_page=30")
  }

  foreach ($release in $releases) {
    $asset = $release.assets | Where-Object { $_.name -match $Pattern } | Select-Object -First 1
    if ($asset) {
      Write-Host "$Repo $($release.tag_name): $($asset.name)"
      $out = Join-Path $env:TEMP $asset.name
      Invoke-WebRequest -Headers $headers -Uri $asset.browser_download_url -OutFile $out
      return $out
    }
  }

  throw "No release in the newest 30 of $Repo carries an asset matching /$Pattern/."
}

function Expand-Into {
  param([string]$Zip, [string]$Target)
  $staging = Join-Path $env:TEMP ([System.IO.Path]::GetRandomFileName())
  Expand-Archive -Path $Zip -DestinationPath $staging -Force
  New-Item -ItemType Directory -Force $Target | Out-Null
  # Release zips vary between a flat layout and a single wrapper directory.
  # Flatten one level so llama-server.exe always lands at bin/llama/.
  $entries = @(Get-ChildItem $staging)
  $root = if ($entries.Count -eq 1 -and $entries[0].PSIsContainer) { $entries[0].FullName } else { $staging }
  Copy-Item -Force -Recurse (Join-Path $root "*") $Target
  Remove-Item -Recurse -Force $staging
}

New-Item -ItemType Directory -Force $Destination | Out-Null

# --- llama.cpp -------------------------------------------------------------
# The CPU build is the one that runs everywhere. Ella asks llama-server for one
# reply at a time, and a classroom machine has no usable GPU to lose.
$llama = Get-ReleaseAsset -Repo "ggml-org/llama.cpp" -Pattern "^llama-.*-bin-win-cpu-x64\.zip$" -Tag $env:ELLA_LLAMA_TAG
Expand-Into -Zip $llama -Target (Join-Path $Destination "bin\llama")

# --- Piper -----------------------------------------------------------------
# The whole archive is extracted on purpose: piper.exe needs its espeak-ng-data
# directory beside it or phonemization fails at synthesis time.
$piper = Get-ReleaseAsset -Repo "rhasspy/piper" -Pattern "^piper_windows_amd64\.zip$" -Tag $env:ELLA_PIPER_TAG
Expand-Into -Zip $piper -Target (Join-Path $Destination "bin\piper")

# --- The voice that ships with the app -------------------------------------
$tts = Join-Path $Destination "models\tts"
New-Item -ItemType Directory -Force $tts | Out-Null

if ($VoiceUrl) {
  Write-Host "Staging the NavGurukul voice from the configured URL"
  Invoke-WebRequest -Uri $VoiceUrl -OutFile (Join-Path $tts "en_IN-navgurukul-medium.onnx")
  Invoke-WebRequest -Uri "$VoiceUrl.json" -OutFile (Join-Path $tts "en_IN-navgurukul-medium.onnx.json")
} else {
  Write-Warning "ELLA_PIPER_VOICE_URL is not set - staging the public en_US voice instead. Ella will not sound Indian in this build."
}

# The stock voice is staged either way, so a bad or half-written custom voice
# still leaves the app able to speak.
$base = "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx"
Invoke-WebRequest -Uri $base -OutFile (Join-Path $tts "en_US-lessac-medium.onnx")
Invoke-WebRequest -Uri "$base.json" -OutFile (Join-Path $tts "en_US-lessac-medium.onnx.json")

# --- What the app will look for at runtime ---------------------------------
$required = @("bin\llama\llama-server.exe", "bin\piper\piper.exe", "models\tts\en_US-lessac-medium.onnx")
foreach ($relative in $required) {
  $path = Join-Path $Destination $relative
  if (-not (Test-Path -PathType Leaf $path)) {
    throw "Staging finished but $relative is missing. The upstream archive layout has probably changed."
  }
}

Write-Host "Staged Windows engines at $Destination"
Get-ChildItem -Recurse $Destination | Measure-Object -Property Length -Sum |
  ForEach-Object { Write-Host ("Total staged size: {0:N0} MB" -f ($_.Sum / 1MB)) }
