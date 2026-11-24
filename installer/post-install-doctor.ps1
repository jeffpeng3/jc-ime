<#
.SYNOPSIS
    安裝完成後執行 jc-tsf-smoke --json，保存診斷記錄。

.DESCRIPTION
    不論結果都寫入 %LOCALAPPDATA%\jc-ime\doctor\：
      <yyyyMMdd-HHmmss>.json  jc-tsf-smoke 的 stdout（jc-ime-doctor/1）
      <yyyyMMdd-HHmmss>.log   stderr 加上人類可讀摘要
      last.txt                這次的摘要，最後一行是 .log 的完整路徑

    有 FAIL 時把 last.txt 複製到剪貼簿，並用 MessageBox 顯示摘要前 15 行與
    log 路徑。全部通過時安靜結束。

    安裝程式以 waituntilterminated 執行這支腳本，所以預設一律以 0 結束，
    避免診斷結果讓安裝失敗。CI 要取得 jc-tsf-smoke 的結束碼時加
    -ExitWithSmokeCode。

.PARAMETER SmokeExe
    jc-tsf-smoke.exe 的路徑。預設找腳本同目錄。

.PARAMETER NoUi
    不碰剪貼簿、不顯示訊息框；CI 與無人環境使用。

.PARAMETER ExitWithSmokeCode
    以 jc-tsf-smoke 的結束碼結束。預設 0，讓 Inno 的 [Run] 不因診斷失敗
    而判定安裝失敗。
#>
#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$SmokeExe,
    [switch]$NoUi,
    [switch]$ExitWithSmokeCode
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

trap {
    if ($ExitWithSmokeCode) { exit 1 }
    exit 0
}

function Write-Utf8NoBom {
    param(
        [Parameter(Mandatory)][string]$Path,
        [Parameter(Mandatory)][AllowEmptyString()][string]$Text
    )
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Text, $encoding)
}

function Get-ReportField {
    param(
        $Object,
        [Parameter(Mandatory)][string]$Name,
        [string]$Default = ''
    )
    if ($null -eq $Object) { return $Default }
    $property = $Object.PSObject.Properties[$Name]
    if ($null -eq $property -or $null -eq $property.Value) { return $Default }
    return [string]$property.Value
}

function Show-DoctorAlert {
    param([Parameter(Mandatory)][string]$Message)
    Add-Type -AssemblyName System.Windows.Forms | Out-Null

    # MessageBox 沒有 TopMost 選項；用一個不可見的 TopMost owner 把它帶到最上層。
    $owner = New-Object System.Windows.Forms.Form
    $owner.TopMost = $true
    $owner.ShowInTaskbar = $false
    $owner.FormBorderStyle = [System.Windows.Forms.FormBorderStyle]::None
    $owner.StartPosition = [System.Windows.Forms.FormStartPosition]::Manual
    $owner.WindowState = [System.Windows.Forms.FormWindowState]::Minimized
    $owner.Show()
    try {
        [void][System.Windows.Forms.MessageBox]::Show(
            $owner,
            $Message,
            'jc 注音 安裝診斷',
            [System.Windows.Forms.MessageBoxButtons]::OK,
            [System.Windows.Forms.MessageBoxIcon]::Warning
        )
    }
    finally {
        $owner.Close()
        $owner.Dispose()
    }
}

$scriptDir = $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($scriptDir)) {
    $scriptDir = (Get-Location).ProviderPath
}
if ([string]::IsNullOrWhiteSpace($SmokeExe)) {
    $SmokeExe = Join-Path $scriptDir 'jc-tsf-smoke.exe'
}

$localAppData = $env:LOCALAPPDATA
if ([string]::IsNullOrWhiteSpace($localAppData)) {
    $localAppData = [Environment]::GetFolderPath('LocalApplicationData')
}
$doctorDir = Join-Path $localAppData 'jc-ime\doctor'
New-Item -ItemType Directory -Path $doctorDir -Force | Out-Null

# 同一秒內重跑時不要蓋掉上一筆；只有撞名才加序號。
$baseStamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$stamp = $baseStamp
$suffix = 1
while ((Test-Path -LiteralPath (Join-Path $doctorDir "$stamp.json")) -or
       (Test-Path -LiteralPath (Join-Path $doctorDir "$stamp.log"))) {
    $stamp = '{0}-{1}' -f $baseStamp, $suffix
    $suffix++
}
$jsonPath = Join-Path $doctorDir "$stamp.json"
$logPath = Join-Path $doctorDir "$stamp.log"
$lastPath = Join-Path $doctorDir 'last.txt'

$exitCode = 0
$stdout = ''
$stderr = ''
$report = $null
$runError = $null
$parseError = $null
$process = $null

try {
    if (-not (Test-Path -LiteralPath $SmokeExe)) {
        throw "找不到 jc-tsf-smoke.exe：$SmokeExe"
    }

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $SmokeExe
    $startInfo.Arguments = '--json'
    $startInfo.WorkingDirectory = $scriptDir
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $startInfo.StandardOutputEncoding = $utf8
    $startInfo.StandardErrorEncoding = $utf8

    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $startInfo
    [void]$process.Start()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $process.WaitForExit()
    $stdout = $stdoutTask.Result
    $stderr = $stderrTask.Result
    $exitCode = $process.ExitCode
}
catch {
    $runError = $_.Exception.Message
    $exitCode = 1
}
finally {
    if ($null -ne $process) {
        $process.Dispose()
    }
}

if ([string]::IsNullOrWhiteSpace($stdout)) {
    if ($null -eq $runError) {
        $runError = 'jc-tsf-smoke 沒有輸出 JSON'
    }
}
else {
    try {
        $report = $stdout | ConvertFrom-Json
    }
    catch {
        $parseError = $_.Exception.Message
    }
    if ($null -eq $report -and $null -eq $parseError) {
        $parseError = 'stdout 不是 JSON 物件'
    }
}

$version = '未知'
$versionProperty = $null
if ($null -ne $report) {
    $versionProperty = $report.PSObject.Properties['version']
}
if ($null -ne $versionProperty -and
    -not [string]::IsNullOrWhiteSpace([string]$versionProperty.Value)) {
    $version = [string]$versionProperty.Value
}
else {
    try {
        if (Test-Path -LiteralPath $SmokeExe) {
            $fileVersion = (Get-Item -LiteralPath $SmokeExe).VersionInfo.ProductVersion
            if (-not [string]::IsNullOrWhiteSpace($fileVersion)) {
                $version = $fileVersion
            }
        }
    }
    catch { }
}

$now = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
$checks = @()
if ($null -ne $report) {
    $checksProperty = $report.PSObject.Properties['checks']
    if ($null -ne $checksProperty -and $null -ne $checksProperty.Value) {
        $checks = @($checksProperty.Value)
    }
}

$summaryLines = @('jc-ime 診斷 {0} {1}' -f $version, $now)
$failCount = 0
$warnCount = 0
foreach ($check in $checks) {
    $status = Get-ReportField -Object $check -Name 'status'
    if ($status -eq 'fail') {
        $failCount++
        $summaryLines += ('[FAIL] {0}：{1}' -f
            (Get-ReportField -Object $check -Name 'title' -Default (Get-ReportField -Object $check -Name 'id')),
            (Get-ReportField -Object $check -Name 'detail'))
    }
    elseif ($status -eq 'warn') {
        $warnCount++
        $summaryLines += ('[WARN] {0}：{1}' -f
            (Get-ReportField -Object $check -Name 'title' -Default (Get-ReportField -Object $check -Name 'id')),
            (Get-ReportField -Object $check -Name 'detail'))
    }
}

if ($null -ne $runError) {
    $failCount++
    $summaryLines += ('[FAIL] 執行診斷：{0}' -f $runError)
}
if ($null -ne $parseError) {
    $failCount++
    $summaryLines += ('[FAIL] 解析 JSON：{0}' -f $parseError)
}
if ($null -ne $report -and $checks.Count -eq 0) {
    $failCount++
    $summaryLines += '[FAIL] 診斷輸出：checks 是空的'
}
if ($exitCode -ne 0 -and $failCount -eq 0) {
    $failCount++
    $summaryLines += ('[FAIL] 結束碼：jc-tsf-smoke 以 {0} 結束' -f $exitCode)
}
if ($failCount -eq 0 -and $warnCount -eq 0) {
    $summaryLines += ('全部通過（{0} 項）' -f $checks.Count)
}
$summaryLines += $logPath
$summary = $summaryLines -join "`r`n"

$stderrText = [string]$stderr
$stderrText = $stderrText.TrimEnd()
$logText = @(
    'jc-ime 診斷'
    ('時間：{0}' -f $now)
    ('版本：{0}' -f $version)
    ('執行檔：{0}' -f $SmokeExe)
    ('結束碼：{0}' -f $exitCode)
    ''
    '== stderr =='
    $stderrText
    ''
    '== 摘要 =='
    $summary
) -join "`r`n"

try {
    Write-Utf8NoBom -Path $jsonPath -Text $stdout.TrimEnd()
}
catch { }
try {
    Write-Utf8NoBom -Path $logPath -Text $logText
}
catch { }
try {
    Write-Utf8NoBom -Path $lastPath -Text $summary
}
catch { }

$hasFailure = ($failCount -gt 0) -or ($exitCode -ne 0)
$canShowUi = -not $NoUi
if ($canShowUi) {
    try {
        $sessionId = (Get-Process -Id $PID).SessionId
        if ($sessionId -eq 0 -or -not [Environment]::UserInteractive) {
            $canShowUi = $false
        }
    }
    catch {
        $canShowUi = $false
    }
}

if ($hasFailure -and $canShowUi) {
    try {
        Get-Content -LiteralPath $lastPath -Raw -Encoding UTF8 | Set-Clipboard
    }
    catch { }

    try {
        $previewLines = @($summaryLines | Select-Object -First 15)
        $message = $previewLines -join "`r`n"
        if ($summaryLines.Count -gt 15) {
            $message += "`r`n`r`nlog：$logPath"
        }
        Show-DoctorAlert -Message $message
    }
    catch { }
}

if ($ExitWithSmokeCode) {
    exit $exitCode
}
exit 0
