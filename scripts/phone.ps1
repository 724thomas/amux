# 자리를 뜨기 전에 폰용 사이드카를 켜는 스크립트입니다. (윈도우판 phone.sh)
#
# 주소를 코드에 박지 않고 매번 알아냅니다. DHCP 임대는 바뀔 수 있고, 노트북은
# 사무실이 아닌 망에 붙을 수도 있기 때문입니다. "지금 실제로 갖고 있는 주소"에만
# 바인딩하면, 열려서는 안 될 곳에서는 애초에 뜨지 않습니다.
#
#   scripts\phone.ps1 -Setup      맨 처음 한 번 — 폰에 발급자(인증서)를 설치
#   scripts\phone.ps1             평소 — 켜고 나갔다가 돌아와서 Ctrl+C
#   scripts\phone.ps1 -Pair       새 폰을 등록할 때 (QR + 6자리 코드)
#   scripts\phone.ps1 -Qr         주소가 바뀌어 폰 북마크를 갱신할 때
#
# 환경변수: AMUX_PHONE_PORT (기본 8000)

[CmdletBinding()]
param(
  # 한 번짜리 발급자 설치 모드. 폰이 아직 발급자를 모르는 상태에서 HTTPS 로 그
  # 발급자를 내려받을 수는 없으니, 이때만 평문으로 뜹니다.
  [switch]$Setup,
  [switch]$Pair,
  [switch]$Qr,
  # 그 외 인자는 amux-phone 에 그대로 넘깁니다.
  [Parameter(ValueFromRemainingArguments = $true)]
  [string[]]$Rest
)

$ErrorActionPreference = 'Stop'

function Die($msg) {
  Write-Host ""
  Write-Host "  $msg" -ForegroundColor Red
  Write-Host ""
  exit 1
}

$Port = if ($env:AMUX_PHONE_PORT) { $env:AMUX_PHONE_PORT } else { '8000' }
$Root = Split-Path -Parent $PSScriptRoot

# amux-phone 이 설정을 찾는 순서와 같아야 합니다 (tls.rs 의 config_base):
# XDG_CONFIG_HOME → HOME\.config → %APPDATA%. 여기서 다른 곳을 보면 "발급자가
# 없다"고 잘못 막게 됩니다.
$ConfigBase =
  if ($env:XDG_CONFIG_HOME) { $env:XDG_CONFIG_HOME }
  elseif ($env:HOME) { Join-Path $env:HOME '.config' }
  else { $env:APPDATA }
$CaCert = Join-Path $ConfigBase 'amux\ca\ca.crt'

# 주소 기록은 설정이 아니라 캐시입니다 — 로밍 프로필에 얹을 이유가 없습니다.
$StateDir = Join-Path $env:LOCALAPPDATA 'amux'
$LastIpFile = Join-Path $StateDir 'phone-last-ip'

# ── 실행 파일 ────────────────────────────────────────────────────────────
$Bin = Join-Path $Root 'target\release\amux-phone.exe'
if (-not (Test-Path $Bin)) { $Bin = Join-Path $Root 'target\debug\amux-phone.exe' }
if (-not (Test-Path $Bin)) {
  Die "amux-phone 이 아직 빌드되지 않았습니다.  cargo build -p amux-phone --release"
}

# ── 지금 이 기기가 밖으로 나갈 때 쓰는 주소 ──────────────────────────────
# 패킷을 보내지 않고 라우팅 표만 물어봅니다.
$Ip = $null
try {
  $Ip = (Find-NetRoute -RemoteIPAddress '1.1.1.1' -ErrorAction Stop)[0].IPAddress
} catch {
  # Find-NetRoute 가 없는 환경(구형/서버 코어)을 위한 대비책. UDP connect 는
  # 패킷을 보내지 않고 경로만 고릅니다 — amux-phone 안의 outbound_ip 와 같은 수법.
  try {
    $udp = New-Object System.Net.Sockets.UdpClient
    $udp.Connect('192.0.2.1', 80)   # TEST-NET-1: 예약 대역, 절대 라우팅되지 않음
    $Ip = $udp.Client.LocalEndPoint.Address.ToString()
    $udp.Close()
  } catch { }
}
if (-not $Ip) { Die "네트워크에 연결되어 있지 않은 것 같습니다." }

# 사설 대역(RFC 1918) 안에서만 뜹니다. 공인 주소로 바인딩하면 인터넷 전체에
# 포트를 여는 셈이고, 발급자도 그 주소는 보증하지 못합니다.
$isTen     = $Ip -like '10.*'
$is192     = $Ip -like '192.168.*'
$is172     = ($Ip -match '^172\.(1[6-9]|2[0-9]|3[01])\.')
if (-not ($isTen -or $is192 -or $is172)) {
  Die "사설망이 아닙니다 (현재 주소 $Ip). 공인 주소로는 띄우지 않습니다."
}
if (-not $isTen) {
  # 사내망은 10.x 입니다. 그 밖의 사설 대역은 집 공유기일 수도, 카페 공용
  # 와이파이일 수도 있습니다 — 둘은 주소만 봐서는 구분되지 않습니다.
  Write-Host ""
  Write-Host "  참고: 사내망(10.x)이 아니라 $Ip 입니다." -ForegroundColor Yellow
  Write-Host "        같은 공유기에 붙은 기기는 이 포트에 닿을 수 있습니다(등록·토큰은 여전히 필요합니다)."
  Write-Host "        믿는 망에서만 켜세요. 카페 같은 공용 와이파이라면 끄는 편이 낫습니다."
}

# ── 이미 떠 있는지 ───────────────────────────────────────────────────────
$busy = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
if ($busy) {
  $who = ($busy | Select-Object -First 1).OwningProcess
  $name = (Get-Process -Id $who -ErrorAction SilentlyContinue).ProcessName
  Die "$Port 포트를 이미 누가 쓰고 있습니다 (PID $who $name).  Stop-Process -Id $who  로 정리하세요."
}

# ── 방화벽이 켜져 있는데 규칙이 없으면 폰이 못 붙습니다 ──────────────────
# 리눅스의 ufw 절과 같은 자리지만 더 자주 걸립니다: 윈도우 방화벽은 기본으로
# 켜져 있고 들어오는 연결을 막습니다. 규칙 조회는 관리자 권한 없이도 됩니다.
$fwOn = @(Get-NetFirewallProfile -ErrorAction SilentlyContinue | Where-Object { $_.Enabled }).Count -gt 0
if ($fwOn) {
  $allowed = Get-NetFirewallPortFilter -ErrorAction SilentlyContinue |
    Where-Object { $_.Protocol -eq 'TCP' -and $_.LocalPort -eq $Port } |
    ForEach-Object { $_ | Get-NetFirewallRule -ErrorAction SilentlyContinue } |
    Where-Object { $_.Enabled -eq 'True' -and $_.Direction -eq 'Inbound' -and $_.Action -eq 'Allow' }
  if (-not $allowed) {
    Write-Host ""
    Write-Host "  참고: 방화벽에 $Port 포트를 여는 규칙이 안 보입니다. 폰에서 안 열리면 이걸 한 번:" -ForegroundColor Yellow
    Write-Host "        (관리자 PowerShell — 지금 붙어 있는 망에서만 열립니다)"
    Write-Host "          New-NetFirewallRule -DisplayName 'amux phone' -Direction Inbound ``"
    Write-Host "            -Action Allow -Protocol TCP -LocalPort $Port -Profile Private"
    Write-Host "        되돌릴 때:  Remove-NetFirewallRule -DisplayName 'amux phone'"
    Write-Host ""
  }
}

# ── 주소가 지난번과 다르면 폰 북마크가 깨집니다 ──────────────────────────
$showQr = @()
New-Item -ItemType Directory -Force -Path $StateDir | Out-Null
if (Test-Path $LastIpFile) {
  $prev = (Get-Content $LastIpFile -Raw).Trim()
  if ($prev -and $prev -ne $Ip) {
    Write-Host "  주소가 지난번과 다릅니다 (이전 $prev → 지금 $Ip)."
    Write-Host "  브라우저는 주소가 다르면 다른 사이트로 보므로 폰에 저장된 등록도 함께 날아갑니다."
    Write-Host "  그래서 QR 과 페어링 코드를 같이 띄웁니다. 아래 QR 을 다시 찍으세요."
    # --qr 만 주면 폰은 코드를 요구하는데 PC 에는 코드가 안 떠서 막다른 골목이 된다.
    $showQr = @('--qr', '--pair')
  }
}
Set-Content -Path $LastIpFile -Value $Ip -NoNewline -Encoding utf8

# 스위치로 받은 것을 그대로 CLI 플래그로 되돌립니다.
$passthru = @()
if ($Pair) { $passthru += '--pair' }
if ($Qr)   { $passthru += '--qr' }
if ($Rest) { $passthru += $Rest }

# ── 실행 ─────────────────────────────────────────────────────────────────
if ($Setup) {
  Write-Host "  발급자 설치 모드입니다. 이 한 번만 평문 HTTP 로 뜹니다."
  Write-Host "  폰에서 열 주소:  http://${Ip}:$Port/ca.crt"
  Write-Host ""
  & $Bin --bind "${Ip}:$Port" --setup-ca --idle-timeout 0 @passthru
  exit $LASTEXITCODE
}

# 발급자가 없으면 HTTPS 로 떠 봐야 폰이 경고를 냅니다.
if (-not (Test-Path $CaCert)) {
  Die "발급자가 아직 없습니다. 먼저 한 번:  scripts\phone.ps1 -Setup"
}

Write-Host "  폰에서 열 주소:  https://${Ip}:$Port"
Write-Host "  이 창을 닫으면 꺼집니다. 30분 동안 아무도 안 쓰면 알아서 닫힙니다."
Write-Host ""

& $Bin --bind "${Ip}:$Port" --tls @showQr @passthru
exit $LASTEXITCODE
