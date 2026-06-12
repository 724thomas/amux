# amux

AI 코딩 에이전트(Claude Code 등)를 **병렬로** 돌리기 위한 Ubuntu 데스크톱 터미널.

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
