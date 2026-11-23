<#
.SYNOPSIS
    建置 jc 注音的可攜 payload 與 Inno Setup setup.exe。

.DESCRIPTION
    步驟：cargo build（x64 全部程式含 jc-tsf，i686 的 jc-tsf，
    以及 jc-tip-register）→ 使用或產生資料 manifest → 暫存 payload（程式、資料、
    授權文件）→ check-payload.py → 選用簽章 → ISCC 編 installer/jc.iss。

    版本號取自 workspace Cargo.toml 的 [workspace.package] version，可以是
    pre-release（例如 0.1.0-rc.1）。setup.exe 的 AppVersion 用完整版本，檔名也用
    完整版本，例如 jc-ime-0.1.0-rc.1-x64-setup.exe；VersionInfoVersion 只吃四段
    數字 x.y.z.N，N 是 rc 序號（正式版為 0），例如 0.1.0-rc.9 是 0.1.0.9。

    安裝包帶一份 TSF 文字服務的 x64 與 i686 兩份：jc-tsf.dll、jc-tsf-x86.dll。
    TSF 依宿主行程的位元數載入對應的那一份；兩份共用同一個 CLSID，理由見
    docs/port/x86-tip.md。舊版 DLL 已移除。

    需要：
      - Visual Studio 2022 的 MSVC 工具鏈，Rust target x86_64-pc-windows-msvc 與
        i686-pc-windows-msvc（rustup target add i686-pc-windows-msvc）
      - Inno Setup 6（ISCC.exe 在 Program Files (x86)\Inno Setup 6 或 PATH）
      - 資料根目錄（預設讀 $env:JC_DATA_ROOT）
      - 選用：Windows SDK 的 signtool.exe（簽章時才需要）

.PARAMETER DataRoot
    資料根目錄，內含 build-zh、english、packs、models 與選用的 manifest.json。
    預設讀環境變數 JC_DATA_ROOT。搭配 -Manifest 時只需保留 shipped: true 的檔案。

.PARAMETER Certificate
    簽章用的 PFX 路徑。預設讀 JC_SIGN_CERT；沒給就輸出未簽章的 setup.exe。

.PARAMETER Manifest
    選用的既有資料 manifest。給定時直接複製這份 manifest，不再用
    `jc-cli data manifest` 重算；適合 CI 只帶 shipped: true 檔案、但仍要保留
    shipped: false 下載項 URL 的情況。預設讀 JC_DATA_MANIFEST。

.PARAMETER IsoVersion
    setup.exe 的 VersionInfoVersion，x.y.z 或 x.y.z.N。預設讀 JC_ISO_VERSION，
    其次是舊的 JC_MSI_VERSION；都沒給時由完整版本推導：正式版是 x.y.z.0，
    x.y.z-rc.N 是 x.y.z.N。給三段時會補上從完整版本推出的 rc 序號，方便既有 CI
    只傳 x.y.z。`-MsiVersion` 保留為別名，舊指令不必改。檔名與顯示版本仍是完整版本。

.EXAMPLE
    .\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data

.EXAMPLE
    # Cargo.toml 是 0.1.0-rc.1：VersionInfoVersion 0.1.0.1，檔名含 -rc.1。
    .\installer\build.ps1 -DataRoot Z:\mnt\work\jc\jc-ime-data -IsoVersion 0.1.0.1
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
    [Alias('MsiVersion')]
    [string]$IsoVersion = $(if ($env:JC_ISO_VERSION) { $env:JC_ISO_VERSION } else { $env:JC_MSI_VERSION }),
    [string]$Iscc = $env:JC_ISCC,
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
    '-p', 'jc-tsf', '-p', 'jc-engine', '-p', 'jc-cli',
    '-p', 'jc-settings-ui', '-p', 'jc-tsf-smoke'
)
$cargoX86Args = @('build') + $profileArgs + @(
    '--target', $TargetX86, '-p', 'jc-tsf'
)
$cargoHelperArgs = @('build') + $profileArgs + @(
    '--target', $Target,
    '--target-dir', $HelperTarget,
    '--manifest-path', (Join-Path $PSScriptRoot 'jc-tip-register\Cargo.toml')
)

function Format-CargoCommand {
    param([Parameter(Mandatory)][string[]]$Arguments)
    return 'cargo ' + ($Arguments -join ' ')
}

# 找檔失敗時列出目錄裡真正存在的 DLL／EXE，命名不符時不必再翻 CI 的整份 log。
function Get-ArtifactNames {
    param([Parameter(Mandatory)][string]$Directory)
    $names = @(Get-ChildItem -Path $Directory -File -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -in '.dll', '.exe' } |
        Select-Object -ExpandProperty Name | Sort-Object)
    if ($names.Count -eq 0) { return '（沒有 .dll 或 .exe）' }
    return ($names -join ', ')
}

# 靜態連結 CRT 的 rustflags 放在 .cargo/config.toml 的兩個 windows-msvc
# target，build.ps1 不再設定 RUSTFLAGS。CI 的 cargo run／cargo build 與這裡
# 共用同一組指紋，避免因環境變數不同而全量重編。
# 出貨條件見 docs/history.md：TSF DLL 不能依賴 vcruntime140.dll。

# 記錄本次建置起點（UTC），稍後 Copy-FreshArtifact 會驗證打包的 DLL／EXE 的
# mtime 晚於這個時間，拒絕 rust-cache 還原、但本次 cargo build 沒重編到的舊檔。
$buildStartedAt = (Get-Date).ToUniversalTime()

# 先建置，再解析輸出目錄。Resolve-ReleaseDir 只驗證既有產物；若在建置前呼叫，
# 乾淨的 runner 上必定找不到 target\<triple>\<profile>（RC1 在 build.ps1:107 失敗）。
Invoke-Checked '建置 workspace 程式' {
    & cargo @cargoX64Args
}

# 32 位元的 TSF DLL。TSF 依宿主行程的位元數載入 TIP，沒有這一份，
# 舊版 Office 等 32 位元程式就看不到輸入法。引擎、CLI 與設定程式只有 x64 版。
Invoke-Checked '建置 x86 jc-tsf' {
    & cargo @cargoX86Args
}

Invoke-Checked '建置 jc-tip-register' {
    & cargo @cargoHelperArgs
}

# 預期輸出只在 target/<triple>/<profile>。絕不退回 target/<profile>：該目錄是
# rust-cache 可能還原的舊產物，曾讓安裝包打包到上一版的 jc_tsf.dll（0.1.2 的
# pipe debug log 因此沒進安裝包，見 fix-msi-stale-dll）。找不到就直接失敗，並
# 帶上剛才實際執行的 cargo 指令與 rustup target add，讓 CI log 能直接行動。
function Resolve-ReleaseDir {
    param(
        [Parameter(Mandatory)][string]$Triple,
        [Parameter(Mandatory)][string]$CargoCommand
    )
    $targeted = Join-Path $TargetRoot (Join-Path $Triple $Configuration)
    if (Test-Path $targeted) { return $targeted }
    throw "找不到建置輸出 $targeted。實際執行：$CargoCommand。請先 rustup target add $Triple，並確認指令有 --target $Triple。"
}

$ReleaseDir = Resolve-ReleaseDir $Target (Format-CargoCommand $cargoX64Args)
$ReleaseDirX86 = Resolve-ReleaseDir $TargetX86 (Format-CargoCommand $cargoX86Args)
Write-Host "==> 使用 x64 建置輸出 $ReleaseDir"
Write-Host "==> 使用 x86 建置輸出 $ReleaseDirX86"

# 從建置輸出複製檔案到 payload，並確認它是本次 cargo build 產生的：
# 1) mtime 必須晚於 $buildStartedAt（容忍 2 秒時鐘誤差），擋掉 rust-cache 還原但
#    本次沒重編到的舊檔；2) 複製後比對 SHA-256，確保 payload 內就是剛驗證過的那份。
function Copy-FreshArtifact {
    param(
        [Parameter(Mandatory)][string]$Source,
        [Parameter(Mandatory)][string]$Destination,
        [Parameter(Mandatory)][datetime]$BuiltAfter,
        [Parameter(Mandatory)][string]$Label
    )
    if (-not (Test-Path $Source)) {
        $directory = Split-Path -Parent $Source
        throw "$Label 不存在：$Source。目錄中實際存在的檔案：$(Get-ArtifactNames $directory)"
    }
    $item = Get-Item $Source
    if ($item.LastWriteTimeUtc -lt $BuiltAfter.AddSeconds(-2)) {
        throw "$Label 的 mtime（$($item.LastWriteTimeUtc.ToString('o'))）早於本次建置起點（$($BuiltAfter.ToString('o'))），可能是 rust-cache 還原的舊檔，拒絕打包。"
    }
    $hash = (Get-FileHash -Algorithm SHA256 $Source).Hash
    Copy-Item $Source $Destination
    $copiedHash = (Get-FileHash -Algorithm SHA256 $Destination).Hash
    if ($hash -ne $copiedHash) {
        throw "$Label 複製到 payload 後 SHA-256 不符（來源 $hash，payload $copiedHash）。"
    }
    Write-Host "    驗證 $Label：mtime $($item.LastWriteTimeUtc.ToString('o'))，SHA-256 $hash"
}

if (-not $DataRoot) {
    throw '沒有資料根目錄：設定 JC_DATA_ROOT 或傳 -DataRoot。'
}
if (-not (Test-Path $DataRoot)) {
    throw "資料根目錄不存在：$DataRoot"
}

# 版本：只認 workspace.package 的 version。完整版本可以有 pre-release 後綴，
# 例如 0.1.0-rc.1；VersionInfoVersion 用四段，第四段是 rc 序號。
$match = Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version\s*=\s*"([^"]+)"' -List
if (-not $match) { throw 'Cargo.toml 找不到 [workspace.package] version' }
$Version = $match.Matches[0].Groups[1].Value
if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$') {
    throw "版本必須是 x.y.z 或 x.y.z-<pre-release>（Cargo.toml 是 $Version）。"
}

$versionParts = $Version -split '-', 2
$rcNumber = 0
if ($versionParts.Count -gt 1) {
    if ($versionParts[1] -match '^rc\.(\d+)$') {
        $rcNumber = [int]$matches[1]
    } else {
        throw "不支援的 pre-release：$($versionParts[1])。目前只認 rc.<序號>（Cargo.toml 是 $Version）。"
    }
}

# CI 到這裡仍只傳三段；補上 rc 序號，讓相鄰 rc 的 VersionInfoVersion 不同。
if (-not $IsoVersion) {
    $IsoVersion = $versionParts[0]
}
if ($IsoVersion -match '^\d+\.\d+\.\d+$') {
    $IsoVersion = "$IsoVersion.$rcNumber"
}
if ($IsoVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') {
    throw "VersionInfoVersion 必須是 x.y.z 或 x.y.z.n（目前是 $IsoVersion）；pre-release 後綴只留在檔名與顯示版本。"
}
Write-Host "==> 發佈版本 $Version（VersionInfoVersion $IsoVersion）"

Write-Host '==> 暫存 payload'
if (Test-Path $Payload) { Remove-Item $Payload -Recurse -Force }
New-Item -ItemType Directory -Path $Payload | Out-Null

# jc-tsf 是 cdylib，Cargo 的 lib target 名稱用底線，產物因此是 jc_tsf.dll；
# payload 內才改成安裝腳本使用的 jc-tsf.dll／jc-tsf-x86.dll。
# 其餘 bin target 保留連字號：jc-engine.exe、jc-cli.exe、jc-settings-ui.exe、
# jc-tsf-smoke.exe，已用 cargo metadata 確認。
$tsfLibName = 'jc_tsf.dll'
$programs = @(
    'jc-engine.exe', 'jc-cli.exe', 'jc-settings-ui.exe', 'jc-tsf-smoke.exe'
)
foreach ($name in $programs) {
    Copy-FreshArtifact -Source (Join-Path $ReleaseDir $name) `
        -Destination (Join-Path $Payload $name) `
        -BuiltAfter $buildStartedAt -Label $name
}

Copy-FreshArtifact -Source (Join-Path $ReleaseDir $tsfLibName) `
    -Destination (Join-Path $Payload 'jc-tsf.dll') `
    -BuiltAfter $buildStartedAt -Label "x64 $tsfLibName"

# x86 的 DLL 也改名進 payload；兩份放在同一個目錄，檔名必須分得出架構。
Copy-FreshArtifact -Source (Join-Path $ReleaseDirX86 $tsfLibName) `
    -Destination (Join-Path $Payload 'jc-tsf-x86.dll') `
    -BuiltAfter $buildStartedAt -Label "x86 $tsfLibName"

# jc-tip-register 用獨立 target-dir，同樣驗證是本次建置的產物。
Copy-FreshArtifact -Source (Join-Path $HelperRelease 'jc-tip-register.exe') `
    -Destination (Join-Path $Payload 'jc-tip-register.exe') `
    -BuiltAfter $buildStartedAt -Label 'jc-tip-register.exe'

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
# 這樣 check-payload.py 可以直接比對雜湊，jc.iss 也照同一組相對路徑引用。
Write-Host '==> 暫存授權文件'
$legalStage = Join-Path $Payload 'legal'
$licenseStage = Join-Path $legalStage 'licenses'
New-Item -ItemType Directory -Path $licenseStage -Force | Out-Null
foreach ($name in @('LICENSE', 'NOTICE')) {
    $source = Join-Path $Root $name
    if (-not (Test-Path $source)) { throw "缺少授權文件 $name（$source）" }
    Copy-Item $source $Payload
}
foreach ($name in @('THIRD-PARTY-NOTICES.md', 'RUST-THIRD-PARTY-NOTICES.md')) {
    $notices = Join-Path $Root "legal\$name"
    if (-not (Test-Path $notices)) {
        throw "缺少授權文件 legal\$name（$notices）"
    }
    Copy-Item $notices $legalStage
}
$licenseFiles = @(Get-ChildItem -Path (Join-Path $Root 'legal\licenses') -File)
if ($licenseFiles.Count -eq 0) {
    throw 'legal\licenses 沒有任何授權全文，拒絕產生安裝包'
}
foreach ($file in $licenseFiles) { Copy-Item $file.FullName $licenseStage }
Write-Host "    授權全文 $($licenseFiles.Count) 份"

$python = Get-Command python -ErrorAction SilentlyContinue
if ($python) {
    # Windows 的 Python 3.12 預設用 cp1252 輸出，腳本印中文會 UnicodeEncodeError。
    # 這兩個變數讓 stdout／stderr 走 UTF-8；腳本本身也有 reconfigure 保護。
    $env:PYTHONUTF8 = '1'
    $env:PYTHONIOENCODING = 'utf-8'
    Invoke-Checked '檢查 jc.iss 引用與 payload 一致' {
        & $python.Source (Join-Path $PSScriptRoot 'check-payload.py') $Payload
    }
} else {
    Write-Warning '找不到 python；跳過 check-payload.py（ISCC 仍會擋掉缺少的檔案）'
}

if (-not $SkipSign -and $Certificate) {
    $signtool = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if (-not $signtool) {
        throw '指定了簽章憑證但找不到 signtool.exe；請安裝 Windows SDK 或加 -SkipSign。'
    }
    $signArgs = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256',
        '/f', $Certificate)
    if ($CertificatePassword) { $signArgs += @('/p', $CertificatePassword) }
    # 兩份 TSF DLL 都要簽章，與 docs/install.md 的簽章清單一致。
    $tsfDllNames = @('jc-tsf.dll', 'jc-tsf-x86.dll')
    foreach ($name in @($programs + $tsfDllNames + 'jc-tip-register.exe')) {
        Invoke-Checked "簽章 $name" { & $signtool.Source @signArgs (Join-Path $Payload $name) }
    }
} else {
    Write-Warning '未指定簽章憑證：setup.exe 與執行檔維持未簽章，SmartScreen 會顯示警告。'
}

# 找 ISCC.exe：-Iscc／JC_ISCC 優先，其次 PATH，最後是 Inno Setup 6 的預設安裝路徑。
if (-not $Iscc) {
    $command = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($command) { $Iscc = $command.Source }
}
if (-not $Iscc) {
    $candidates = @()
    if (${env:ProgramFiles(x86)}) {
        $candidates += (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe')
    }
    if ($env:ProgramFiles) {
        $candidates += (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe')
    }
    if ($env:LOCALAPPDATA) {
        $candidates += (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    }
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            $Iscc = $candidate
            break
        }
    }
}
if (-not $Iscc -or -not (Test-Path $Iscc)) {
    throw '找不到 ISCC.exe；請安裝 Inno Setup 6（choco install innosetup -y）或傳 -Iscc。'
}

New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$setup = Join-Path $OutDir "jc-ime-$Version-x64-setup.exe"
Invoke-Checked '建置 setup.exe' {
    & $Iscc "/DMyAppVersion=$Version" `
        "/DIsoVersion=$IsoVersion" `
        "/DPayloadDir=$Payload" `
        "/O$OutDir" `
        (Join-Path $PSScriptRoot 'jc.iss')
}
if (-not (Test-Path $setup)) {
    throw "ISCC 結束但找不到預期的 setup.exe：$setup"
}

if (-not $SkipSign -and $Certificate) {
    $signtool = Get-Command signtool.exe
    $signArgs = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256',
        '/f', $Certificate)
    if ($CertificatePassword) { $signArgs += @('/p', $CertificatePassword) }
    Invoke-Checked '簽章 setup.exe' { & $signtool.Source @signArgs $setup }
}

$size = [math]::Round((Get-Item $setup).Length / 1MB, 1)
Write-Host ''
Write-Host "完成：$setup（$size MiB）"
Write-Host "    發佈版本 $Version；VersionInfoVersion $IsoVersion"
Write-Host '安裝：jc-ime-<version>-x64-setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /LOG=install.log'
Write-Host '移除："C:\Program Files\jc-ime\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART'
