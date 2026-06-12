# amux

AI 코딩 에이전트(Claude Code 등)를 **병렬로** 돌리기 위한 Ubuntu 데스크톱 터미널.

<스크린샷 1 — 메인 화면 전체: 워크스페이스 2~3개가 사이드바에 보이고, 오른쪽은 2개 이상 분할된 pane. 한 pane에는 Claude Code가 실행 중, 다른 pane에는 빌드/일반 셸. 사이드바에 터미널별 상태 칩·git 브랜치·cwd·포트 칩(:5173 등)이 보이는 상태>
<!-- ![메인 화면](docs/screenshots/01-overview.png) -->

- 워크스페이스(탭) × 분할 pane — 동시 다중 터미널, 개수 제한 없음
- 에이전트 상태 칩: 🔴 processing / 🟢 processed / 🔵 idle / 🟡 waiting
- 데스크톱 알림 + 사이드바 알림 히스토리 (BEL, OSC 9/777, Claude Code hook)
- 사이드바에 브랜치 · cwd · 리슨 포트(클릭하면 브라우저 오픈) 표시
- 모든 조작이 마우스로 가능 (분할·닫기·이름변경·드래그 재배치·테마)
- `amux` CLI로 외부 자동화: `ls`, `split`, `send`, `read-screen`, `notify` …
- 색 테마 6종 (Tokyo Night 기본)

스택: Tauri 2 (Rust) + Svelte 5 + xterm.js.

## 설치 (.deb)

```bash
sudo apt install ./amux_0.1.0_amd64.deb
```

- GNOME 앱 목록에 **amux** 아이콘 등록, `amux` CLI는 `/usr/bin/amux`로 설치
- 다른 Ubuntu PC에는 `.deb` 파일 하나만 복사해서 동일하게 설치
- 의존성(webkit2gtk 등)은 apt가 자동 해결

빈 화면이 뜨면 (WebKitGTK + Wayland DMABUF 이슈):

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 amux-app
```

## 사용법

### 화면 구성

왼쪽 **사이드바**(워크스페이스 → 터미널 목록 / 알림 히스토리 / 테마·폰트) +
오른쪽 **터미널 영역**(분할 pane). 사이드바 폭은 경계선 드래그로 조절.

### 마우스 (모든 조작 가능)

| 하고 싶은 것 | 방법 |
|---|---|
| 새 워크스페이스 | 사이드바 `+ 새 워크스페이스` |
| 워크스페이스/터미널 전환 | 사이드바 항목 클릭 (키보드 포커스 자동 이동) |
| 분할 | pane에 마우스 올리면 우상단 툴바 ◫(오른쪽) ⬓(아래) |
| pane 재배치 | 툴바 ⠿를 드래그 → 다른 pane의 상/하/좌/우에 드롭 |
| 크기 조절 | 분할선 드래그, 더블클릭하면 50:50 |
| 이름 변경 / 닫기 | 사이드바 항목 우클릭 |
| 복사 | 텍스트 드래그하면 자동 복사 (우클릭 메뉴도 있음) |
| 붙여넣기 | 휠 클릭 또는 Ctrl+V |
| 폰트 크기 | Ctrl+휠 또는 사이드바 하단 Aa − ＋ |
| 테마 | 사이드바 하단 테마 드롭다운 |
| 서버 열기 | 사이드바 포트 칩(`:5173`) 클릭 → 브라우저 |
| URL 열기 | 터미널 안 링크 Ctrl+클릭 |

<스크린샷 2 — pane 드래그 재배치 순간: pane에 마우스를 올려 우상단 호버 툴바(⠿ ◫ ⬓ ✕)가 보이고, ⠿를 드래그해 다른 pane 위에 올려 파란 드롭 존 오버레이(상/하/좌/우 절반 하이라이트)가 표시된 상태>
<!-- ![드래그 재배치](docs/screenshots/02-drag-rearrange.png) -->

### 키보드

| 키 | 동작 |
|---|---|
| Ctrl+Shift+T | 새 워크스페이스 |
| Ctrl+Shift+D / S | 오른쪽 / 아래 분할 |
| Ctrl+Shift+W | pane 닫기 |
| Alt+방향키 | pane 간 이동 |
| Ctrl+PgUp / PgDn | 워크스페이스 전환 |
| Shift+Enter | 줄바꿈 (Claude Code 입력창 포함) |
| Shift+방향키 | 커서 기준 텍스트 선택 |
| Ctrl+C / X / V | 복사(선택 시) / 잘라내기(선택 시) / 붙여넣기 |
| Ctrl+Shift+F | 스크롤백 검색 |
| Ctrl+= / − / 0 | 폰트 크기 |

### 에이전트 상태 칩

사이드바의 터미널마다 상태가 항상 표시됩니다:

| 칩 | 의미 |
|---|---|
| 🔴 processing… | 작업 진행 중 |
| 🟢 processed | 작업 완료, 아직 안 봄 |
| 🔵 idle | 한가함 (완료 확인됨) |
| 🟡 waiting | 입력 대기 (예: Claude 권한 질문) |

Claude Code는 hook 연동 시 정확하게 동작하고(아래 참고), 일반 명령은
출력 휴리스틱으로 자동 판정됩니다.

<스크린샷 3 — 상태 칩 4색이 동시에 보이는 사이드바 클로즈업: 터미널 4개가 각각 processing…(빨강) / processed(초록) / idle(파랑) / waiting(노랑) 칩을 단 상태. waiting pane에는 알림 뱃지(파란 점)와 알림 메시지 텍스트("Claude needs your permission..." 류)가 함께 보이면 베스트>
<!-- ![상태 칩](docs/screenshots/03-status-chips.png) -->

### 알림

보고 있지 않은 pane에서 작업 완료·입력 대기가 발생하면 GNOME 데스크톱
알림 + pane 하이라이트 + 사이드바 뱃지가 옵니다. 사이드바 하단
**알림 히스토리**에서 과거 알림을 클릭하면 해당 pane으로 이동합니다.

<스크린샷 4 — 알림이 도착한 순간: 화면 상단에 GNOME 데스크톱 알림 토스트("작업이 끝났습니다" 등)가 떠 있고, 해당 pane 테두리에 하늘색 하이라이트 링이 깜빡이는 중이며, 사이드바 하단 알림 히스토리 패널에 시간·pane 이름·메시지 목록이 쌓여 있는 상태>
<!-- ![알림](docs/screenshots/04-notifications.png) -->

### 테마

사이드바 하단 드롭다운에서 6종 선택: Tokyo Night(기본) · Dracula ·
Catppuccin Mocha · Gruvbox Dark · Nord · Solarized Light.
터미널 16색 팔레트와 앱 전체 색이 함께 바뀌고 자동 저장됩니다.

<스크린샷 5 — 테마 드롭다운이 열린 모습: 사이드바 하단 테마 메뉴에 6개 항목이 색 견본과 함께 보이고, Tokyo Night가 아닌 다른 테마(예: Solarized Light나 Gruvbox Dark)가 적용된 화면이면 효과가 잘 드러남>
<!-- ![테마](docs/screenshots/05-themes.png) -->

## Claude Code 상태 연동 (권장)

```bash
python3 scripts/install-claude-hooks.py
```

`~/.claude/settings.json`에 hook을 병합 설치합니다 (백업 자동 생성).
자세한 매핑은 [docs/claude-hooks.md](docs/claude-hooks.md) 참고.

## CLI

pane 안에서는 인자 없이 자기 pane을 가리킵니다 (`AMUX_PANE_ID` 상속).

```bash
amux ls                          # 워크스페이스/pane 목록
amux ws create                   # 새 워크스페이스
amux split --right               # 현재 pane 오른쪽 분할
amux send 'git status' --enter   # 텍스트 입력
amux send-keys C-c               # 키 입력 (tmux 스타일 이름)
amux read-screen p-3fa2c1        # 다른 pane 화면 읽기
amux notify --kind done --title 빌드 --body 완료
```

소켓: `$XDG_RUNTIME_DIR/amux/amux.sock`, NDJSON JSON-RPC 2.0 (`socat`으로 디버깅 가능).

## 개발

```bash
# 요구: rustup, bun, libwebkit2gtk-4.1-dev 등 Tauri 의존성
bun install
bun run tauri dev        # 개발 실행 (HMR)
cargo test --workspace   # Rust 테스트
bun run tauri build      # 릴리스 + .deb 번들
```

> ⚠️ 패키징 전 `cargo build --release -p amux-cli` 필요 — `.deb`이
> `target/release/amux`를 `/usr/bin/amux`로 동봉합니다.

구조: `crates/amux-protocol`(공유 타입) / `crates/amux-core`(엔진: PTY·터미널 상태·알림·소켓 서버) / `crates/amux-cli`(CLI) / `src-tauri`(앱 셸) / `src`(Svelte UI).
