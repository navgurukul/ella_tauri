param(
  [Parameter(Mandatory = $true)][string]$EngineRoot
)

$ErrorActionPreference = "Stop"
$ProjectDir = Split-Path -Parent $PSScriptRoot
$Destination = Join-Path $ProjectDir "src-tauri\resources\engines"
$Required = @(
  "bin\llama\llama-server.exe",
  "bin\whisper\whisper-server.exe",
  "bin\piper\piper.exe",
  "models\llm\model.gguf",
  "models\stt\canary-180m-flash-Q8_0.gguf",
  "models\stt\ggml-small.bin",
  "models\tts\en_US-lessac-medium.onnx",
  "models\tts\en_US-lessac-medium.onnx.json"
)

foreach ($Relative in $Required) {
  $Source = Join-Path $EngineRoot $Relative
  if (-not (Test-Path -PathType Leaf $Source)) {
    throw "Cannot stage Windows bundle; missing $Source"
  }
}

New-Item -ItemType Directory -Force $Destination | Out-Null
foreach ($Folder in @("bin\llama", "bin\whisper", "bin\piper", "models\llm", "models\stt", "models\tts")) {
  $Source = Join-Path $EngineRoot $Folder
  $Target = Join-Path $Destination $Folder
  New-Item -ItemType Directory -Force $Target | Out-Null
  Copy-Item -Force -Recurse (Join-Path $Source "*") $Target
}

Write-Host "Windows x86-64 engines staged at $Destination"
Write-Host "Build with: npm run tauri -- build --target x86_64-pc-windows-msvc"
