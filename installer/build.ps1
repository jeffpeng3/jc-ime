<#
.SYNOPSIS
    建置 jc 注音的可攜 payload 與 WiX v5 MSI。

.DESCRIPTION
    步驟：cargo build（x64 全部程式 + i686 的 jc-tsf，以及 jc-tip-register）→ 使用或
    產生資料 manifest → 暫存 payload（程式、資料、授權文件）→ wix build → 選用簽章。

    版本號取自 workspace Cargo.toml 的 [workspace.package] version，可以是
    pre-release（例如 0.1.0-rc.1）。MSI 的 ProductVersion 只接受 x.y.z，所以
    另外用 -MsiVersion 指定；沒給時由完整版本去掉 -pre-release 推導。MSI 檔名
    一律用完整版本，例如 jc-ime-0.1.0-rc.1-x64.msi。

    安裝包同時帶兩個架構的 TSF 文字服務：x64 的 jc-tsf.dll 與 i686 的
    jc-tsf-x86.dll。TSF 依宿主行程的位元數載入對應的那一份，兩份各自註冊一個
    CLSID，理由見 docs/port/x86-tip.md。

    需要：
      - Visual Studio 2022 的 MSVC 工具鏈，Rust target x86_64-pc-windows-msvc 與
        i686-pc-windows-msvc（rustup target add i686-pc-windows-msvc）
      - .NET 6 以上與 WiX v5：dotnet tool install --global wix --version 5.*
      - 資料根目錄（預設讀 $env:JC_DATA_ROOT）
      - 選用：Windows SDK 的 signtool.exe（簽章時才需要）

.PARAMETER DataRoot
    資料根目錄，內含 build-zh、english、packs、models 與選用的 manifest.json。
    預設讀環境變數 JC_DATA_ROOT。搭配 -Manifest 時只需保留 shipped: true 的檔案。

.PARAMETER Certificate
    簽章用的 PFX 路徑。預設讀 JC_SIGN_CERT；沒給就輸出未簽章的 MSI。

.PARAMETER Manifest
    選用的既有資料 manifest。給定時直接複製這份 manifest，不再用
    `jc-cli data manifest` 重算；適合 CI 只帶 shipped: true 檔案、但仍要保留
    shipped: false 下載項 URL 的情況。預設讀 JC_DATA_MANIFEST。

.PARAMETER MsiVersion
    MSI 的 ProductVersion，必須是 x.y.z。預設讀 JC_MSI_VERSION；兩者都沒給時由
    完整版本推導，也就是去掉 `-rc.1` 這類 pre-release 後綴。只用於 MSI 內部
    版本與升級判斷，檔名與顯示版本仍是完整版本。

.EXAMPLE
    .\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data

.EXAMPLE
    # Cargo.toml 是 0.1.0-rc.1：MSI ProductVersion 0.1.0，檔名含 -rc.1。
    .\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data -MsiVersion 0.1.0
#>

# 說明區塊必須是檔案的第一個元素，Get-Help 才讀得到，所以 #Requires 放在它
# 之後；PowerShell 允許 #Requires 出現在任何位置。
#Requires -Version 5.1

[CmdletBinding()]
param(
    [string]$DataRoot = $env:JC_DATA_ROOT,
    [string]$Configuration = 'release',
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string]$TargetX86 = 'i686-pc-windows-msvc',
    [string]$TimestampUrl = 'http://timestamp.digicert.com',
    [string]$Certificate = $env:JC_SIGN_CERT,
    [string]$CertificatePassword = $env:JC_SIGN_PASSWORD,
    [string]$Manifest = $env:JC_DATA_MANIFEST,
    [string]$MsiVersion = $env:JC_MSI_VERSION,
    [switch]$SkipSign
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Invoke-Checked {
    param(
        [Parameter(Mandatory)][string]$Label,
        [Parameter(Mandatory)][scriptblock]$Action
    )
    Write-Host "==> $Label"
    & $Action
    $exitCode = Get-Variable LASTEXITCODE -ValueOnly -ErrorAction SilentlyContinue
    if ($null -ne $exitCode -and $exitCode -ne 0) {
        throw "$Label 失敗（結束碼 $exitCode）"
    }
}

$Root = Split-Path -Parent $PSScriptRoot
$TargetRoot = Join-Path $Root 'target'
$OutDir = Join-Path $TargetRoot 'installer'
$Payload = Join-Path $OutDir 'payload'
$HelperTarget = Join-Path $TargetRoot 'tip-register'
$HelperRelease = Join-Path $HelperTarget "$Target\$Configuration"

$profileArgs = @()
if ($Configuration -eq 'release') {
    $profileArgs = @('--release')
} else {
    $profileArgs = @('--profile', $Configuration)
}

# 實際執行的 cargo 參數只定義一次，建置與錯誤訊息共用，避免兩者不一致。
$cargoX64Args = @('build') + $profileArgs + @(
    '--target', $Target,
    '-p', 'jc-tsf', '-p', 'jc-engine', '-p', 'jc-cli', '-p', 'jc-settings-ui', '-p', 'jc-tsf-smoke'
)
$cargoX86Args = @('build') + $profileArgs + @('--target', $TargetX86, '-p', 'jc-tsf')
$cargoHelperArgs = @('build') + $profileArgs + @(
    '--target', $Target,
    '--target-dir', $HelperTarget,
    '--manifest-path', (Join-Path $PSScriptRoot 'jc-tip-register\Cargo.toml')
)

function Format-CargoCommand {
    param([Parameter(Mandatory)][string[]]$Arguments)
    return 'cargo ' + ($Arguments -join ' ')
}

# 靜態連結 CRT：TSF DLL 會被載入 explorer 與任意程式，不能依賴 vcruntime140.dll。
# 這是 docs/history.md 記錄的出貨條件。
$env:RUSTFLAGS = (@($env:RUSTFLAGS, '-C target-feature=+crt-static') |
    Where-Object { $_ }) -join ' '

# 先建置，再解析輸出目錄。Resolve-ReleaseDir 只驗證既有產物；若在建置前呼叫，
# 乾淨的 runner 上必定找不到 target\<triple>\<profile>（RC1 在 build.ps1:107 失敗）。
Invoke-Checked '建置 workspace 程式' {
    & cargo @cargoX64Args
}

# 32 位元的 TSF DLL。TSF 依宿主行程的位元數載入 TIP，沒有這一份舊版 Office 等
# 32 位元程式就看不到輸入法。引擎、CLI 與設定程式只有 x64 版。
Invoke-Checked '建置 x86 jc-tsf' {
    & cargo @cargoX86Args
}

Invoke-Checked '建置 jc-tip-register' {
    & cargo @cargoHelperArgs
}

# 預期輸出在 target/<triple>/<profile>；找不到時退回 target/<profile>，並明確說出
# 實際使用哪一個路徑，避免 CI 因 cargo build 沒帶 --target 而默默拿錯檔案。
# 只讓 x64 退回：target/<profile> 的產物是 x64，拿來當 x86 DLL 會裝錯架構。
# 錯誤訊息帶上剛才實際執行的 cargo 指令與 rustup target add，讓 CI log 能直接行動。
function Resolve-ReleaseDir {
    param(
        [Parameter(Mandatory)][string]$Triple,
        [Parameter(Mandatory)][string]$CargoCommand,
        [switch]$AllowFallback
    )
    $targeted = Join-Path $TargetRoot (Join-Path $Triple $Configuration)
    if (Test-Path $targeted) { return $targeted }
    if ($AllowFallback) {
        $fallback = Join-Path $TargetRoot $Configuration
        if (Test-Path $fallback) {
            Write-Warning "找不到 $targeted，退回 $fallback。"
            return $fallback
        }
    }
    throw "找不到建置輸出 $targeted。實際執行：$CargoCommand。請先 rustup target add $Triple，並確認指令有 --target $Triple。"
}

$ReleaseDir = Resolve-ReleaseDir $Target (Format-CargoCommand $cargoX64Args) -AllowFallback
$ReleaseDirX86 = Resolve-ReleaseDir $TargetX86 (Format-CargoCommand $cargoX86Args)
Write-Host "==> 使用 x64 建置輸出 $ReleaseDir"
Write-Host "==> 使用 x86 建置輸出 $ReleaseDirX86"

if (-not $DataRoot) {
    throw '沒有資料根目錄：設定 JC_DATA_ROOT 或傳 -DataRoot。'
}
if (-not (Test-Path $DataRoot)) {
    throw "資料根目錄不存在：$DataRoot"
}

# 版本：只認 workspace.package 的 version。完整版本可以有 pre-release 後綴，
# 例如 0.1.0-rc.1；MSI ProductVersion 另外處理。
$match = Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' -List
if (-not $match) { throw 'Cargo.toml 找不到 [workspace.package] version' }
$Version = $match.Matches[0].Groups[1].Value
if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
    throw "版本必須是 x.y.z 或 x.y.z-<pre-release>（Cargo.toml 是 $Version）。"
}

if (-not $MsiVersion) {
    $MsiVersion = ($Version -split '-', 2)[0]
}
if ($MsiVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "MSI ProductVersion 必須是 x.y.z（目前是 $MsiVersion）；pre-release 後綴只留在檔名與顯示版本。"
}
Write-Host "==> 發佈版本 $Version（MSI ProductVersion $MsiVersion）"

Write-Host '==> 暫存 payload'
if (Test-Path $Payload) { Remove-Item $Payload -Recurse -Force }
New-Item -ItemType Directory -Path $Payload | Out-Null

$programs = @(
    'jc-tsf.dll', 'jc-engine.exe', 'jc-cli.exe', 'jc-settings-ui.exe', 'jc-tsf-smoke.exe'
)
foreach ($name in $programs) {
    $source = Join-Path $ReleaseDir $name
    if (-not (Test-Path $source)) { throw "建置輸出缺少 $name（$source）" }
    Copy-Item $source $Payload
}

# x86 的 DLL 改名進 payload：兩份都叫 jc-tsf.dll，安裝後放在同一個目錄，
# 檔名必須分得出來，WXS 才引用得到兩個檔案。
$x86Dll = Join-Path $ReleaseDirX86 'jc-tsf.dll'
if (-not (Test-Path $x86Dll)) {
    throw "建置輸出缺少 x86 jc-tsf.dll（$x86Dll）；請先 rustup target add $TargetX86"
}
Copy-Item $x86Dll (Join-Path $Payload 'jc-tsf-x86.dll')

Copy-Item (Join-Path $HelperRelease 'jc-tip-register.exe') $Payload

$dataStage = Join-Path $Payload 'data'
New-Item -ItemType Directory -Path $dataStage | Out-Null

$cli = Join-Path $ReleaseDir 'jc-cli.exe'
$manifestPath = Join-Path $dataStage 'manifest.json'
if ($Manifest) {
    if (-not (Test-Path $Manifest)) { throw "指定的資料 manifest 不存在：$Manifest" }
    Invoke-Checked '複製既有資料 manifest' { Copy-Item $Manifest $manifestPath -Force }
} else {
    Invoke-Checked '產生資料 manifest' {
        & $cli data manifest $DataRoot --out $manifestPath
    }
}

$manifestData = Get-Content $manifestPath -Raw | ConvertFrom-Json
$shipped = @($manifestData.files | Where-Object { $_.shipped })
if ($shipped.Count -eq 0) { throw 'manifest 沒有任何 shipped: true 的檔案' }
foreach ($entry in $shipped) {
    $relative = $entry.path -replace '/', '\'
    $source = Join-Path $DataRoot $relative
    $destination = Join-Path $dataStage $relative
    if (-not (Test-Path $source)) { throw "manifest 列出的檔案不存在：$source" }
    New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
    Copy-Item $source $destination
}
Invoke-Checked '驗證暫存資料' {
    & $cli data verify $dataStage --manifest $manifestPath
}

# 授權文件：payload 內的路徑與 repo 相同（LICENSE、NOTICE、legal/...），
# 這樣 check-payload.py 可以直接比對雜湊，MSI 也照同一組相對路徑引用。
Write-Host '==> 暫存授權文件'
$legalStage = Join-Path $Payload 'legal'
$licenseStage = Join-Path $legalStage 'licenses'
New-Item -ItemType Directory -Path $licenseStage -Force | Out-Null
foreach ($name in @('LICENSE', 'NOTICE')) {
    $source = Join-Path $Root $name
    if (-not (Test-Path $source)) { throw "缺少授權文件 $name（$source）" }
    Copy-Item $source $Payload
}
$notices = Join-Path $Root 'legal\THIRD-PARTY-NOTICES.md'
if (-not (Test-Path $notices)) {
    throw "缺少授權文件 legal\THIRD-PARTY-NOTICES.md（$notices）"
}
Copy-Item $notices $legalStage
$licenseFiles = @(Get-ChildItem -Path (Join-Path $Root 'legal\licenses') -File)
if ($licenseFiles.Count -eq 0) {
    throw 'legal\licenses 沒有任何授權全文，拒絕產生安裝包'
}
foreach ($file in $licenseFiles) { Copy-Item $file.FullName $licenseStage }
Write-Host "    授權全文 $($licenseFiles.Count) 份"

$python = Get-Command python -ErrorAction SilentlyContinue
if ($python) {
    Invoke-Checked '檢查 WXS 引用與 payload 一致' {
        & $python.Source (Join-Path $PSScriptRoot 'check-payload.py') $Payload
    }
} else {
    Write-Warning '找不到 python；跳過 check-payload.py（wix build 仍會擋掉缺少的檔案）'
}

if (-not $SkipSign -and $Certificate) {
    $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if (-not $signtool) {
        throw '指定了簽章憑證但找不到 signtool.exe；請安裝 Windows SDK 或加 -SkipSign。'
    }
    $signArgs = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256',
        '/f', $Certificate)
    if ($CertificatePassword) { $signArgs += @('/p', $CertificatePassword) }
    foreach ($name in @($programs + 'jc-tsf-x86.dll' + 'jc-tip-register.exe')) {
        Invoke-Checked "簽章 $name" { & $signtool.Source @signArgs (Join-Path $Payload $name) }
    }
} else {
    Write-Warning '未指定簽章憑證：MSI 與執行檔維持未簽章，SmartScreen 會顯示警告。'
}

$wix = Get-Command wix.exe -ErrorAction SilentlyContinue
if (-not $wix) {
    throw '找不到 wix.exe；請執行 dotnet tool install --global wix --version 5.*'
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$msi = Join-Path $OutDir "jc-ime-$Version-x64.msi"
Invoke-Checked '建置 MSI' {
    & $wix.Source build -arch x64 `
        -d "Version=$MsiVersion" `
        -d "PayloadDir=$Payload" `
        -o $msi `
        (Join-Path $PSScriptRoot 'jc-ime.wxs')
}

if (-not $SkipSign -and $Certificate) {
    $signtool = Get-Command signtool.exe
    $signArgs = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256',
        '/f', $Certificate)
    if ($CertificatePassword) { $signArgs += @('/p', $CertificatePassword) }
    Invoke-Checked '簽章 MSI' { & $signtool.Source @signArgs $msi }
}

$size = [math]::Round((Get-Item $msi).Length / 1MB, 1)
Write-Host ''
Write-Host "完成：$msi（$size MiB）"
Write-Host "    發佈版本 $Version；MSI ProductVersion $MsiVersion"
Write-Host '安裝：msiexec /i "jc-ime.msi" /l*v install.log'
Write-Host '移除：msiexec /x "jc-ime.msi" /l*v uninstall.log'
