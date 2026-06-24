# amux

AI 코딩 에이전트(Claude Code 등)를 **병렬로** 돌리기 위한 Ubuntu 데스크톱 터미널.

<img width="1856" height="1080" alt="image" src="https://github.com/user-attachments/assets/0f635231-9b41-4945-ad60-8dd0414e8d6f" />


- 워크스페이스(탭) × 분할 pane — 동시 다중 터미널, 개수 제한 없음
- 에이전트 상태 칩: 🔴 processing / 🟢 processed / 🔵 idle / 🟡 waiting
- 데스크톱 알림 + 사이드바 알림 히스토리 (BEL, OSC 9/777, Claude Code hook)
- 사이드바에 브랜치 · cwd · 리슨 포트(클릭하면 브라우저 오픈) 표시
- 모든 조작이 마우스로 가능 (분할·닫기·이름변경·드래그 재배치·테마)
- `amux` CLI로 외부 자동화: `ls`, `split`, `send`, `read-screen`, `notify` …
- 색 테마 38종 (Tokyo Night 기본 · 다크/라이트)

스택: Tauri 2 (Rust) + Svelte 5 + xterm.js.

## 변경 내역

### v0.3.0
- **색 테마 32종 추가 (총 38종)** — Catppuccin Latte/Frappé/Macchiato, Rosé Pine(+Moon/Dawn), Everforest, Kanagawa, Ayu, One Dark/Light, Monokai Pro, Tokyo Night Storm/Day, Solarized Dark, GitHub Dark/Light, Night Owl, Nightfox, Synthwave Alpha, Cobalt2 등 (다크 24 · 라이트 8)
- **알림·상태 UX 개선**
  - 워크스페이스 탭의 "작업이 끝났습니다" 문구 제거 (🟢 processed 칩으로 충분)
  - 🟡 waiting pane을 포커스하면 idle로 해제 (이전엔 waiting으로 남던 버그)
  - 🔴 processing 칩 애니메이션 (`processing.` → `..` → `...`)
  - 알림 패널: 해당 pane을 확인하면 그 pane 알림이 자동으로 사라짐 (누적 방지)
  - 알림 패널 높이를 드래그로 조절 (워크스페이스 목록과 공간 배분)

### v0.2.0
- dock 아이콘에 processed 카운트 배지

### v0.1.0
- 최초 릴리스 — 워크스페이스·분할 pane, 에이전트 상태, 알림, CLI

## 설치

[**Releases**](https://github.com/724thomas/amux/releases)에서 최신 `.deb`를 받아 설치합니다:

```bash
wget https://github.com/724thomas/amux/releases/download/v0.3.0/amux_0.3.0_amd64.deb
sudo apt install ./amux_0.3.0_amd64.deb
```

- GNOME 앱 목록에 **amux** 아이콘 등록, `amux` CLI는 `/usr/bin/amux`로 설치
- 의존성(webkit2gtk 등)은 apt가 자동 해결
- 빌드 도구 없이 설치 파일 하나로 끝 — 소스 빌드는 아래 [개발](#개발) 참고

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

<img width="1846" height="1072" alt="image" src="https://github.com/user-attachments/assets/77ee872f-81ee-46f8-bb4b-8614405b5a4e" />


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

<img width="1846" height="1072" alt="image" src="https://github.com/user-attachments/assets/1812209b-cf5c-43de-ac5a-8a1f8b88f7c5" />


### 테마

사이드바 하단 드롭다운에서 **38종** 선택 — Tokyo Night(기본) · Dracula ·
Catppuccin(Mocha/Latte/Frappé/Macchiato) · Gruvbox · Nord · Solarized ·
One Dark/Light · Rosé Pine · Everforest · Kanagawa · Ayu · Monokai Pro ·
GitHub · Night Owl · Synthwave Alpha 등 (다크/라이트).
터미널 16색 팔레트와 앱 전체 색이 함께 바뀌고 자동 저장됩니다.

<img width="168" height="221" alt="image" src="https://github.com/user-attachments/assets/656cc4c4-9581-4f42-8235-c276346ab5ff" />


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
