# amux — Ubuntu/Debian 설치·실행 가이드 (Linux의 Claude에게 붙여넣는 문서)

> **이 문서는 Ubuntu/Debian에서 실행 중인 Claude Code(또는 Claude)에게 통째로 붙여넣어
> 자동으로 설치·빌드·실행하도록 만든 핸드오프 문서입니다.**
> Ubuntu는 amux의 **원조·기준 플랫폼**입니다 — 빌드·테스트가 모두 통과(green)하는 곳이라,
> 가장 신뢰도가 높습니다. 설치 경로는 두 가지: **(A) 미리 빌드된 `.deb`** (가장 간단),
> **(B) 소스에서 빌드**. 특별한 이유가 없으면 (A)를 먼저 시도하세요.

---

## 0. Linux Claude에게 — 먼저 읽을 것

- **스택**: Tauri 2 (Rust 백엔드) + Svelte 5 + xterm.js (프론트엔드). PTY는 `portable-pty`,
  CLI↔앱 통신은 `interprocess`(Unix 도메인 소켓)로 처리합니다.
- **메타데이터**: pane의 cwd·git 브랜치·리슨 포트는 `/proc`에서 읽습니다(리눅스 네이티브
  경로). 그래서 사이드바 정보가 전부 채워집니다.
- **사람의 승인이 필요한 단계**: `apt`/`dpkg` 설치는 `sudo`가 필요합니다.
- **소스 확보**: 소스 빌드 시 `main`을 clone 하면 됩니다.

---

## 경로 A — 미리 빌드된 `.deb`로 설치 (가장 간단)

### A-1. 받기 & 설치

[Releases](https://github.com/724thomas/amux/releases)에서 최신 `.deb`를 받아 설치합니다.

```bash
wget https://github.com/724thomas/amux/releases/download/v0.5.0/amux_0.5.0_amd64.deb
sudo apt install ./amux_0.5.0_amd64.deb
```

- GNOME 앱 목록에 **amux** 아이콘이 등록되고, `amux` CLI는 **`/usr/bin/amux`** 로 설치됩니다.
- 의존성(webkit2gtk 등)은 apt가 자동 해결합니다.
- 빌드 도구 없이 설치 파일 하나로 끝. 소스 빌드는 경로 B.

### A-2. 빈 화면이 뜨면 (WebKitGTK + Wayland DMABUF 이슈)

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 amux-app
```

항상 이 옵션으로 뜨게 하려면 `.desktop` 파일이나 셸 alias에 넣으세요.

경로 A로 설치했다면 [§5 실행](#5-실행--amux-cli)·[§6 hooks](#6-claude-code-상태-연동-hooks-선택-권장)로 건너뛰면 됩니다.

---

## 경로 B — 소스에서 빌드

### 1. 사전 도구 설치

```bash
# Tauri 2 리눅스 시스템 의존성 (Ubuntu 22.04+/Debian 12+ 기준)
sudo apt update
sudo apt install -y \
  libwebkit2gtk-4.1-dev \
  build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
  pkg-config

# Rust (rustup)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"

# Bun (프론트엔드 빌드 도구)
curl -fsSL https://bun.sh/install | bash
# 안내대로 새 셸을 열거나 PATH 반영
```

> 배포판/버전에 따라 패키지명이 다를 수 있습니다(예: 구형은 `libwebkit2gtk-4.0-dev`).
> Tauri 공식 문서의 "Linux prerequisites"를 기준으로 맞추세요.

### 2. 소스 받기

```bash
git clone https://github.com/724thomas/amux.git
cd amux
```

### 3. 1단계 — 빠른 실행(dev) 스모크 테스트  ← 여기부터 시작

```bash
bun install
bun run tauri dev
```

- 처음엔 Rust 의존성 컴파일로 몇 분 걸립니다.
- 성공하면 amux 창이 뜹니다. **실행하자마자 워크스페이스가 열리지 않습니다("빈 시작").**
  왼쪽 사이드바의 **`+ 새 워크스페이스`** 로 제목을 입력하면 그때 셸 pane이 열립니다.
  (이전 세션이 있으면 "지난 세션이 남아 있습니다" 복구 카드가 뜹니다.)
- 빈 창(사이드바도 안 보임)만 뜨면 §A-2의 DMABUF 옵션을 시도하세요.

### 4. 자가검증 (테스트)

```bash
source "$HOME/.cargo/env"
cargo build --workspace
cargo test --workspace
```

- `pane::tests::echo_round_trip` — 실제 셸을 띄워 출력을 읽음(PTY 런타임 증거).
- `server::tests::ipc_round_trip` — Unix 소켓으로 앱↔CLI 왕복.
- `engine::tests::session_round_trip_*` — 세션 저장/복구.
- Ubuntu에서는 전 워크스페이스 테스트가 그린이어야 정상입니다.

### 5-B. 패키지 설치본(.deb) 만들기 (선택)

```bash
cargo build --release -p amux-cli   # target/release/amux (CLI) 생성 — ⚠️ 먼저!
bun run tauri build                 # 릴리스 + .deb 번들
```

- ⚠️ `tauri build` 전에 **반드시** `cargo build --release -p amux-cli`를 먼저 — `.deb`이
  `target/release/amux`를 `/usr/bin/amux`로 동봉합니다(`tauri.conf.json`의 `linux.deb.files`).
- 산출물: `target/release/bundle/deb/amux_0.5.0_amd64.deb`. 이걸 `sudo apt install ./…`로
  설치하면 경로 A와 같은 상태가 됩니다.

---

## 5. 실행 & `amux` CLI

- **앱 실행**: 앱 목록에서 amux, 또는 터미널에서 `amux-app` (빈 화면이면 §A-2).
- **CLI**: `.deb`로 설치했다면 `amux`가 `/usr/bin/amux`에 있어 바로 동작합니다.
  ```bash
  amux --version                  # 0.5.0
  amux ls
  amux ws create --tab-name 작업
  amux send 'git status' --enter --pane <pane-id>
  ```

> 소켓 경로: `$XDG_RUNTIME_DIR/amux/amux.sock`
> (`socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/amux/amux.sock`로 디버깅 가능).

---

## 6. Claude Code 상태 연동 hooks (선택, 권장)

```bash
python3 scripts/install-claude-hooks.py
```

- `~/.claude/settings.json`에 hook을 병합 설치(기존 설정 백업).
- hook은 `amux notify ...` 형태이며, 앱에 연결 못 하면 조용히 종료합니다.
- 이미 돌던 Claude 세션은 재시작해야 hook 설정을 다시 읽습니다.

---

## 7. 폰 사이드카 (선택) — Ubuntu 네이티브

`scripts/phone.sh`는 **Ubuntu 전용으로 설계**되었습니다(사내망에서 폰으로 amux를 보는 기능).
`ip route`·`ss`·`ufw`에 의존하고, 사내망(`10.0.0.0/8`)에서만 뜨도록 가드가 걸려 있습니다.

```bash
cargo build --release -p amux-phone
scripts/phone.sh --setup      # 최초 1회: 폰에 발급자(인증서) 설치
scripts/phone.sh              # 평소: 켜고 나갔다가 Ctrl+C
scripts/phone.sh --pair       # 새 폰 등록 (QR + 6자리 코드)
```

자세한 사용법은 스크립트 상단 주석을 참고하세요.

---

## 8. 플랫폼 참고 — 왜 Ubuntu가 기준인가

- pane의 **cwd·리슨 포트**를 `/proc/<pid>/cwd`, `/proc/net/tcp{,6}`, `/proc/<pid>/fd`에서
  읽습니다(`meta/cwd.rs`, `meta/ports.rs`의 리눅스 분기). 이게 원본 구현이고, macOS는
  같은 정보를 libproc로 새로 구현했습니다(`macos_install.md` §8 참고).
- **데스크톱 알림**은 DBus(`org.freedesktop.Notifications`)로 나가며 Wayland/X11에서
  동일하게 동작합니다. 우선순위(Critical/Normal/Low)도 반영됩니다.
- Dock 배지(미확인 완료 개수)는 GNOME Dock의 Unity LauncherEntry로 표시됩니다.

---

## 9. 문제 해결 (자주 나는 에러)

| 증상 | 원인 / 해결 |
|---|---|
| 앱 창이 **하얗게/빈 화면** | WebKitGTK+Wayland DMABUF → `WEBKIT_DISABLE_DMABUF_RENDERER=1 amux-app` (§A-2) |
| `error: failed to run custom build command for ... webkit2gtk` | `libwebkit2gtk-4.1-dev` 등 시스템 의존성 누락 → §1 |
| `bun: command not found` | bun PATH 미반영 → 새 셸 (§1) |
| `cargo: command not found` | `source "$HOME/.cargo/env"` (§1) |
| `.deb` 설치 시 의존성 오류 | `sudo apt install ./파일.deb` (apt가 의존성 해결, `dpkg -i`는 안 함) |
| pane에서 `amux: command not found` | 소스 빌드만 하고 `.deb` 설치 안 함 → `.deb` 설치하거나 `target/release/amux`를 PATH에 |
| hook이 안 먹음 | ① `amux` PATH ② Claude Code 세션 재시작 |

---

## 10. Linux Claude를 위한 한 줄 요약(작업 순서)

- **간단히**: [Releases](https://github.com/724thomas/amux/releases)의 `.deb` → `sudo apt install ./amux_*.deb` (경로 A). 빈 화면이면 §A-2.
- **소스 빌드**: §1 도구 → §2 clone → `bun install && bun run tauri dev`(§3) → `cargo test --workspace`(§4) → 필요 시 §5-B `.deb`, §6 hooks.
- 문제가 막히면, 어떤 단계에서 어떤 에러가 났는지 그대로 사용자에게 보고하세요.
