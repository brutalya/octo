# Dev (Windows): API server on 127.0.0.1:7878 plus Vite dev server (proxies /api).
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
cargo build -p octo-server
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
$server = Start-Process -FilePath 'target\debug\octo-server.exe' -ArgumentList $args -NoNewWindow -PassThru
try {
    Set-Location web
    npm run dev
} finally {
    if (-not $server.HasExited) { taskkill /T /F /PID $server.Id | Out-Null }
}
