#!/usr/bin/env bash
# 자리를 뜨기 전에 폰용 사이드카를 켜는 스크립트입니다.
#
# 주소를 코드에 박지 않고 매번 알아냅니다. DHCP 임대는 바뀔 수 있고, 노트북은
# 사무실이 아닌 망에 붙을 수도 있기 때문입니다. "지금 실제로 갖고 있는 주소"에만
# 바인딩하면, 열려서는 안 될 곳에서는 애초에 뜨지 않습니다.
#
#   scripts/phone.sh --setup      맨 처음 한 번 — 폰에 발급자(인증서)를 설치
#   scripts/phone.sh              평소 — 켜고 나갔다가 돌아와서 Ctrl+C
#   scripts/phone.sh --pair       새 폰을 등록할 때 (QR + 6자리 코드)
#   scripts/phone.sh --qr         주소가 바뀌어 폰 북마크를 갱신할 때
#
# 환경변수: AMUX_PHONE_PORT (기본 8000)

set -euo pipefail

PORT="${AMUX_PHONE_PORT:-8000}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STATE_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/amux"
LAST_IP_FILE="$STATE_DIR/phone-last-ip"
CA_CERT="${XDG_CONFIG_HOME:-$HOME/.config}/amux/ca/ca.crt"

die() { printf '\n  %s\n\n' "$*" >&2; exit 1; }

# --setup 은 인증서를 설치하기 위한 한 번짜리 모드라 평문으로 뜹니다. 폰이
# 아직 발급자를 모르는 상태에서 HTTPS 로 그 발급자를 내려받을 수는 없으니까요.
SETUP=""
ARGS=()
for a in "$@"; do
  case "$a" in
    --setup) SETUP=1 ;;
    *) ARGS+=("$a") ;;
  esac
done

# ── 실행 파일 ────────────────────────────────────────────────────────────
BIN="$ROOT/target/release/amux-phone"
[ -x "$BIN" ] || BIN="$ROOT/target/debug/amux-phone"
[ -x "$BIN" ] || die "amux-phone 이 아직 빌드되지 않았습니다.  cargo build -p amux-phone --release"

# ── 지금 이 기기가 밖으로 나갈 때 쓰는 주소 ──────────────────────────────
# 패킷을 보내지 않고 라우팅 표만 물어봅니다.
IP="$(ip -4 route get 1.1.1.1 2>/dev/null \
      | awk '{for (i = 1; i <= NF; i++) if ($i == "src") { print $(i + 1); exit } }')"
[ -n "$IP" ] || die "네트워크에 연결되어 있지 않은 것 같습니다."

# 사내망은 10.0.0.0/8 입니다. 카페나 집(대개 192.168.x)에서 실수로 포트를 여는
# 사고를 여기서 막습니다 — 방화벽 규칙도 10.8.0.0/24 로만 열려 있습니다.
case "$IP" in
  10.*) ;;
  *) die "회사망이 아닙니다 (현재 주소 $IP). 사내에서만 실행하세요." ;;
esac

# ── 이미 떠 있는지 ───────────────────────────────────────────────────────
if ss -ltn 2>/dev/null | grep -q "[:.]$PORT[[:space:]]"; then
  die "$PORT 포트를 이미 누가 쓰고 있습니다.  ss -ltnp | grep :$PORT  로 확인하고 kill 하세요."
fi

# ── 방화벽이 켜져 있는데 규칙이 없으면 폰이 못 붙습니다 ──────────────────
# 규칙 목록은 sudo 없이는 못 읽으므로, 최근에 차단된 기록이 있으면 알려만 줍니다.
if grep -qi '^ENABLED=yes' /etc/ufw/ufw.conf 2>/dev/null; then
  if [ -r /var/log/ufw.log ] &&
     tail -n 2000 /var/log/ufw.log 2>/dev/null | grep -q "DPT=$PORT "; then
    printf '  참고: 최근에 %s 포트로 오던 접속이 방화벽에 막힌 기록이 있습니다.\n' "$PORT"
    printf '        안 되면:  sudo ufw allow from 10.8.0.0/24 to any port %s proto tcp\n\n' "$PORT"
  fi
fi

# ── 주소가 지난번과 다르면 폰 북마크가 깨집니다 ──────────────────────────
SHOW_QR=""
mkdir -p "$STATE_DIR"
if [ -f "$LAST_IP_FILE" ]; then
  PREV="$(cat "$LAST_IP_FILE")"
  if [ "$PREV" != "$IP" ]; then
    printf '  주소가 지난번과 다릅니다 (이전 %s → 지금 %s).\n' "$PREV" "$IP"
    printf '  폰 북마크가 안 열릴 테니 아래 QR 을 다시 찍으세요.\n'
    SHOW_QR="--qr"
  fi
fi
printf '%s' "$IP" > "$LAST_IP_FILE"

# ── 실행 ─────────────────────────────────────────────────────────────────
if [ -n "$SETUP" ]; then
  printf '  발급자 설치 모드입니다. 이 한 번만 평문 HTTP 로 뜹니다.\n'
  printf '  폰에서 열 주소:  http://%s:%s/ca.crt\n\n' "$IP" "$PORT"
  exec "$BIN" --bind "$IP:$PORT" --setup-ca --idle-timeout 0 "${ARGS[@]}"
fi

# 발급자가 없으면 HTTPS 로 떠 봐야 폰이 경고를 냅니다.
[ -f "$CA_CERT" ] || die "발급자가 아직 없습니다. 먼저 한 번:  scripts/phone.sh --setup"

printf '  폰에서 열 주소:  https://%s:%s\n' "$IP" "$PORT"
printf '  이 창을 닫으면 꺼집니다. 30분 동안 아무도 안 쓰면 알아서 닫힙니다.\n\n'

exec "$BIN" --bind "$IP:$PORT" --tls ${SHOW_QR:+$SHOW_QR} "${ARGS[@]}"
