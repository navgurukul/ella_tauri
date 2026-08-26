param(
  [string]$EngineRoot = "",
  [int]$LlmPort = 39091,
  [int]$WhisperPort = 39092
)

$ErrorActionPreference = "Stop"
$ProjectDir = Split-Path -Parent $PSScriptRoot
if (-not $EngineRoot) {
  $EngineRoot = [IO.Path]::GetFullPath((Join-Path $ProjectDir "..\..\ella_app\build\engines"))
}

$Llama = Join-Path $EngineRoot "bin\llama\llama-server.exe"
$Whisper = Join-Path $EngineRoot "bin\whisper\whisper-server.exe"
$LlmModel = Join-Path $EngineRoot "models\llm\model.gguf"
$WhisperModel = Join-Path $EngineRoot "models\stt\ggml-small.bin"
$CanaryModel = Join-Path $EngineRoot "models\stt\canary-180m-flash-Q8_0.gguf"

foreach ($Required in @($Llama, $Whisper, $LlmModel, $WhisperModel, $CanaryModel)) {
  if (-not (Test-Path -PathType Leaf $Required)) {
    throw "Missing engine asset: $Required. Install STT models with: py -3 ..\fetch_models.py --dest ..\..\ella_app\build\engines --only stt,stt_fallback"
  }
}
$Header = New-Object byte[] 4
$Stream = [IO.File]::OpenRead($CanaryModel)
try { [void]$Stream.Read($Header, 0, 4) } finally { $Stream.Dispose() }
if ([Text.Encoding]::ASCII.GetString($Header) -ne "GGUF") {
  throw "Canary model has an invalid GGUF header: $CanaryModel"
}

$LlamaProcess = $null
$WhisperProcess = $null
try {
  $LlamaProcess = Start-Process -PassThru -NoNewWindow -FilePath $Llama -ArgumentList @(
    "--model", $LlmModel, "--host", "127.0.0.1", "--port", $LlmPort,
    "--ctx-size", "4096", "--threads", "4", "--parallel", "1"
  )
  $WhisperProcess = Start-Process -PassThru -NoNewWindow -FilePath $Whisper -ArgumentList @(
    "--model", $WhisperModel, "--host", "127.0.0.1", "--port", $WhisperPort,
    "--threads", "3", "--language", "en"
  )

  for ($Attempt = 1; $Attempt -le 180; $Attempt++) {
    try {
      Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$LlmPort/health" | Out-Null
      Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:$WhisperPort/health" | Out-Null
      break
    } catch {
      if ($LlamaProcess.HasExited -or $WhisperProcess.HasExited) {
        throw "A local engine exited before becoming healthy. Inspect its console output above."
      }
      if ($Attempt -eq 180) { throw "Local engines did not become healthy within three minutes." }
      Start-Sleep -Seconds 1
    }
  }

  $env:ELLA_ENGINE_MODE = "local"
  $env:ELLA_ENGINE_ROOT = $EngineRoot
  $env:ELLA_LLM_BASE_URL = "http://127.0.0.1:$LlmPort/v1"
  $env:ELLA_STT_BASE_URL = "http://127.0.0.1:$WhisperPort"
  Push-Location $ProjectDir
  try { npm run desktop:dev } finally { Pop-Location }
} finally {
  foreach ($Process in @($WhisperProcess, $LlamaProcess)) {
    if ($Process -and -not $Process.HasExited) { Stop-Process -Id $Process.Id -Force }
  }
}
