# 使用 PFX 证书为单个 Windows PE 文件签名，并立即验证签名与时间戳。
#
# 设计原理（Why）：主程序必须先于 NSIS 打包完成签名，安装器才能在解包后保留
# 有效的 Authenticode 签名；安装器本身还需在生成后再次签名。脚本把同一套
# 签名与验证口径收敛到一处，避免工作流里出现两段逐渐漂移的命令。
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateNotNullOrEmpty()]
    [string]$FilePath,

    [string]$TimestampUrl = "http://timestamp.digicert.com"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $FilePath -PathType Leaf)) {
    throw "待签名文件不存在: $FilePath"
}
if ([string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PATH) -or
    [string]::IsNullOrWhiteSpace($env:WINDOWS_CERTIFICATE_PASSWORD)) {
    throw "缺少 Authenticode 证书路径或密码环境变量"
}

$signTool = (Get-Command signtool.exe -ErrorAction SilentlyContinue | Select-Object -First 1).Source
if ([string]::IsNullOrWhiteSpace($signTool)) {
    $kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
    if (Test-Path -LiteralPath $kitsRoot) {
        $signTool = Get-ChildItem -LiteralPath $kitsRoot -Filter signtool.exe -File -Recurse `
            -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' } |
            Sort-Object LastWriteTime -Descending |
            Select-Object -First 1 -ExpandProperty FullName
    }
}
if ([string]::IsNullOrWhiteSpace($signTool)) {
    throw "未找到 Windows SDK 中的 signtool.exe"
}

Write-Host "签署 Authenticode 文件: $FilePath"
& $signTool sign `
    /fd SHA256 `
    /td SHA256 `
    /tr $TimestampUrl `
    /f $env:WINDOWS_CERTIFICATE_PATH `
    /p $env:WINDOWS_CERTIFICATE_PASSWORD `
    /d "Transom" `
    $FilePath
if ($LASTEXITCODE -ne 0) {
    throw "Authenticode 签名失败，退出码: $LASTEXITCODE，文件: $FilePath"
}

& $signTool verify /pa /all /v $FilePath
if ($LASTEXITCODE -ne 0) {
    throw "signtool 验证失败，退出码: $LASTEXITCODE，文件: $FilePath"
}

$signature = Get-AuthenticodeSignature -LiteralPath $FilePath
if ($signature.Status -ne "Valid") {
    throw "Authenticode 状态不是 Valid，实际状态: $($signature.Status)，文件: $FilePath"
}

Write-Host "Authenticode 签名与时间戳验证通过: $FilePath"
