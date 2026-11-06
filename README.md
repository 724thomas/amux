# amux

AI 코딩 에이전트(Claude Code 등)를 여러 개 **병렬로** 돌리기 위한 데스크톱 터미널입니다. Ubuntu, macOS, Windows 에서 돕니다.

<img width="1856" height="1080" alt="image" src="https://github.com/user-attachments/assets/0f635231-9b41-4945-ad60-8dd0414e8d6f" />

스택: Tauri 2 (Rust) + Svelte 5 + xterm.js

## 주요 기능

**터미널 정리**
- 워크스페이스 > 탭 > 탭 안 분할 pane. 터미널 개수 제한이 없고, 안 보이는 탭도 계속 돌아갑니다.
- 워크스페이스와 탭 배치, 각 pane 의 Claude 대화가 저장되어 다음 실행에서 한 번에 복구됩니다.
- 하단 고정 입력창: 긴 출력을 위로 올려 읽는 중에도 화면이 튀지 않고 다음 프롬프트를 씁니다.
- 직전 명령 칩: pane 마다 마지막으로 보낸 명령을 고정해 보여 줍니다.

**에이전트 지켜보기**
- 상태 칩: 🔴 작업 중, 🟢 끝남(안 봄), 🔵 한가함, 🟡 입력 대기, 🟣 DONE(직접 고정하는 검토 중)
- 데스크톱 알림과 사이드바의 **지금 봐야 할 에이전트** 목록
- 사이드바에 브랜치, 현재 폴더, 리슨 포트(누르면 브라우저로 열림)
- Mission Control 대시보드, 브로드캐스트 입력, 명령 팔레트, 지난 Claude 대화 검색
- Claude 구독 한도(5시간, 주간)를 amux 가 5분마다 조회해 모든 세션이 같은 숫자를 보게 함

**Claude 대화 화면 (Ctrl+Shift+M)**
- 터미널의 Claude Code 를 웹 채팅처럼 쓰고, 답에 그림(mermaid, svg)을 그립니다.
- 고치는 파일이 옆 편집기에서 입력되듯 실시간으로 보입니다. 파일 창을 별도 팝업 창으로 모을 수 있습니다.
- 서브에이전트가 생기면 오른쪽 칸에 카드로 보입니다(터미널 모드에서도).
- 터미널과 대화 화면은 같은 Claude 세션을 이어 씁니다.

**자동화**
- `amux` CLI: `ls`, `tab new/close/focus/rename`, `split`, `send`, `read-screen`, `notify` 등
- 색 테마 38종(Tokyo Night 기본, 다크와 라이트)

## 설치

[**Releases**](https://github.com/724thomas/amux/releases) 에서 OS 에 맞는 설치본을 받습니다. OS 별 상세 안내는 [`ubuntu_install.md`](ubuntu_install.md), [`macos_install.md`](macos_install.md), [`windows_install.md`](windows_install.md) 에 있습니다. 각 문서는 그 OS 에서 도는 Claude Code 에게 통째로 붙여 넣으면 설치, 빌드, 실행까지 해 주도록 쓴 문서입니다.

### 리눅스 (Ubuntu/Debian)

```bash
wget https://github.com/724thomas/amux/releases/download/v0.5.0/amux_0.5.0_amd64.deb
sudo apt install ./amux_0.5.0_amd64.deb
```

GNOME 앱 목록에 **amux** 가 등록되고, `amux` CLI 는 `/usr/bin/amux` 에 설치됩니다. 빈 화면이 뜨면(WebKitGTK + Wayland DMABUF 문제) 이렇게 실행합니다.

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 amux-app
```

### 윈도우 (10/11)

Releases 의 `amux_0.5.0_x64-setup.exe` 를 실행합니다. CLI 를 PATH 에 넣는 법과 Claude 훅 설정은 [`windows_install.md`](windows_install.md) 를 봅니다. 윈도우 설치본은 `v*` 태그를 푸시하면 GitHub Actions 가 만들어 같은 릴리스에 붙입니다.

### macOS (Apple Silicon/Intel)

소스에서 빌드합니다. Rust([rustup](https://rustup.rs)), bun(`brew install oven-sh/bun/bun`), Xcode Command Line Tools(`xcode-select --install`)를 깔고 실행합니다.

```bash
scripts/install-macos.sh               # amux.app 은 /Applications, amux CLI 는 /usr/local/bin
scripts/install-macos.sh --build-only  # 빌드만 (target/release/bundle/)
```

서명 없는 앱이라 첫 실행은 우클릭 > **열기**, 또는 `xattr -dr com.apple.quarantine /Applications/amux.app`. 자세한 것은 [`macos_install.md`](macos_install.md).

## 사용법

### 화면 구성

왼쪽 **사이드바**(워크스페이스와 탭 목록, 지금 봐야 할 에이전트, 테마와 글자 크기)와 오른쪽 **터미널 영역**(위에 탭바, 아래에 그 탭의 분할 pane)으로 나뉩니다.

- amux 는 빈 상태로 시작합니다. `+ 새 워크스페이스`(또는 `Ctrl+Shift+N`)를 누르면 워크스페이스 이름, 첫 탭 이름을 차례로 묻습니다. 비우고 Enter 를 치면 `워크스페이스 N`, `탭 1` 로 붙습니다.
- **탭**은 화면 한 장입니다. 이름은 탭에만 있고, 탭 안에서 나눈 칸은 `작업 #1`, `작업 #2` 처럼 순번으로 구분합니다. 새 탭도 제목부터 묻습니다(`Ctrl+T`, `Enter` 두 번이면 빈 탭).
- 탭이나 워크스페이스, 창을 닫으면 확인 창이 먼저 뜹니다. 그 안에서 돌던 Claude 도 함께 끝나기 때문입니다.
- 지난 세션이 남아 있으면 시작 화면에 복구 카드가 뜹니다. 배치와 pane 별 폴더를 되살리고, Claude 대화가 있던 pane 에는 `claude --resume <id>` 를 대신 쳐 줍니다(아래 Claude Code 상태 연동 필요).

### Claude 대화 화면

pane 에서 `Ctrl+Shift+M`(명령 팔레트의 "Claude 채팅 열기")을 누르면 그 pane 이 Claude 대화 화면으로 바뀝니다. 다시 누르거나 "터미널로" 를 누르면 터미널로 돌아옵니다.

- **같은 세션을 이어 씁니다.** 터미널에서 돌던 Claude 가 입력을 기다리는 중이면 amux 가 그 Claude 를 끝내고 대화 화면이 같은 세션을 이어받습니다. 그때까지의 대화는 위에 "터미널에서 하던 대화" 로 보입니다. 터미널로 돌아가면 셸에 `claude --resume <id>` 를 쳐서 같은 대화를 엽니다. Claude 가 일하는 중이거나 승인을 기다리는 중이면 넘기지 않고 알려 줍니다.
- **파일 탭.** Claude 가 파일을 고치기 시작하면 오른쪽에 읽기 전용 편집기가 저절로 열리고, 파일 전체를 띄운 채 지워질 줄은 취소선, 새 글은 초록으로 입력되듯 보여 줍니다. 기본 비율은 파일 3 : 대화 2 이고, 왼쪽 가장자리를 끌어 바꿉니다(두 번 누르면 3:2). 줄은 바꾸지 않고 가로 막대로 넘기며, **따라가기**는 입력되는 위치를 가로로도 쫓아갑니다. ✕ 로 닫으면 다음 메시지 전까지 닫혀 있습니다.
- **파일 팝업 창.** 파일 탭 머리의 ⧉ 로 "amux 파일 보기" 창에 옮깁니다. 여러 세션은 칸으로 나뉘고, 칸을 두 번 누르면 그 칸만 크게 봅니다. 팝업 창이 열려 있으면 다른 대화의 파일 변경도 저절로 그 창으로 갑니다.
- **서브에이전트 칸.** Claude 가 서브에이전트를 띄우면 pane 이 3:2 로 나뉘고 오른쪽에 카드가 쌓입니다. 맡은 일, 지금 하는 단계, 편집 내용, 마지막 보고가 보이고, 끝난 카드는 다음 메시지 때 닫힙니다. 터미널 모드에서도 같습니다.
- 터미널 모드에도 pane 툴바의 📂 로 파일 창을 열 수 있습니다. 다만 입력 중 모습은 없고, 도구 호출이 끝날 때마다 바뀐 내용을 다시 보여 줍니다.

### 하단 입력창

pane 맨 아래의 입력칸에 쓰는 동안에는 터미널에 아무 키도 들어가지 않아 읽던 스크롤이 그대로입니다. `Enter` 보내기, `Shift+Enter` 줄바꿈, `Ctrl+Enter` 제출 없이 입력만, `Esc` 터미널로, `Ctrl+Shift+E` 입력창과 터미널 오가기. 입력칸이 비어 있을 때 `/` `!` `#` `@` `↑` `↓` `Tab` 은 터미널로 바로 넘어가 Claude Code 의 자동완성과 메뉴가 그대로 동작합니다. Claude 가 권한이나 선택을 물으면 입력칸이 노랗게 바뀝니다.

### 마우스

| 하고 싶은 것 | 방법 |
|---|---|
| 새 워크스페이스 | 사이드바 `+ 새 워크스페이스` > 이름 Enter > 첫 탭 이름 Enter |
| 새 탭 | 탭바 오른쪽 `+` > 제목 입력 후 Enter |
| 탭 전환 / 이름 변경 / 순서 변경 / 닫기 | 클릭 / 더블클릭 / 드래그 / `×` 또는 휠 클릭 |
| 탭 안 분할 | pane 에 마우스를 올리면 상단 가운데 툴바의 ◫(오른쪽) ⬓(아래) |
| pane 재배치 | 툴바 ⠿ 를 끌어 같은 탭 안 다른 pane 의 상하좌우에 놓기 |
| 크기 조절 | 분할선 드래그, 더블클릭하면 50:50 |
| 이름 변경 / 닫기 | 사이드바 항목 우클릭 |
| 복사 / 붙여넣기 | 끌어서 선택하면 자동 복사 / 휠 클릭 또는 Ctrl+V |
| 글자 크기 | Ctrl+휠 또는 사이드바 하단 Aa − ＋ |
| 직전 명령 칩, 하단 입력창 켜고 끄기 | 사이드바 하단 토글 |
| 테마 | 사이드바 하단 드롭다운 |
| 서버 열기 / URL 열기 | 사이드바 포트 칩(`:5173`) 클릭 / 터미널 안 링크 Ctrl+클릭 |

<img width="1846" height="1072" alt="image" src="https://github.com/user-attachments/assets/77ee872f-81ee-46f8-bb4b-8614405b5a4e" />

### 키보드

| 키 | 동작 |
|---|---|
| Ctrl+T / Ctrl+W | 새 탭 / 탭 닫기 (확인 창, Enter 로 확인) |
| Ctrl+Tab / Ctrl+Shift+Tab | 다음 / 이전 탭 |
| Alt+1…9 | N번째 탭 |
| Ctrl+Shift+N / Ctrl+Shift+W | 새 워크스페이스 / 워크스페이스 닫기 |
| Ctrl+PgUp / PgDn, Ctrl+Shift+1…9 | 워크스페이스 전환 / N번째 워크스페이스 |
| Ctrl+Shift+D / S | 탭 안에서 오른쪽 / 아래 분할 |
| Alt+방향키 | 탭 안 pane 사이 이동 |
| Ctrl+Shift+M | Claude 대화 화면 ↔ 터미널 (같은 세션을 이어 씀) |
| Ctrl+Shift+B | 브로드캐스트 (현재 탭의 pane 에 동시 입력) |
| Ctrl+Shift+P | 명령 팔레트 |
| Ctrl+Shift+H | 지난 Claude 대화 검색 |
| Ctrl+Shift+A | Mission Control 대시보드 |
| Ctrl+Shift+E | 하단 입력창 ↔ 터미널 |
| Ctrl+Shift+F | 스크롤백 검색 |
| Shift+Enter | 줄바꿈 (Claude Code 입력창, 하단 입력창 모두) |
| Ctrl+C / X / V | 복사(선택 시) / 잘라내기(선택 시) / 붙여넣기 |
| Ctrl+= / − / 0 | 글자 크기 |

### 에이전트 상태 칩

사이드바의 탭마다, 그리고 탭바의 색 점으로 상태가 늘 보입니다. 탭 안에 터미널이 여럿이면 가장 급한 상태가 탭의 상태가 됩니다(🟡 → 🟢 → 🔴 → 🔵 → 🟣 순).

| 칩 | 뜻 |
|---|---|
| 🔴 processing | 작업 중 |
| 🟢 processed | 작업이 끝났고 아직 안 봄 |
| 🔵 idle | 한가함 (끝난 것을 확인함) |
| 🟡 waiting | 입력 대기 (예: Claude 권한 질문) |
| 🟣 DONE | 검토 중. pane 툴바 맨 왼쪽 ✓ 로 직접 고정 |

앞의 넷은 amux 가 자동으로 판정합니다. Claude Code 는 훅을 깔면 정확하고(아래), 일반 명령은 출력 모양으로 짐작합니다. DONE 은 고정된 동안 무엇도 바꾸지 못하고, ✓ 를 다시 누르면 풀립니다. 재시작하면 사라집니다.

<img width="1846" height="1072" alt="image" src="https://github.com/user-attachments/assets/1812209b-cf5c-43de-ac5a-8a1f8b88f7c5" />

### 테마

사이드바 하단 드롭다운에서 38종을 고릅니다(Tokyo Night 기본, Dracula, Catppuccin, Gruvbox, Nord, Solarized, One Dark/Light, Rosé Pine, Everforest, Kanagawa, GitHub 등). 터미널 16색과 앱 전체 색이 함께 바뀌고 저장됩니다.

<img width="168" height="221" alt="image" src="https://github.com/user-attachments/assets/656cc4c4-9581-4f42-8235-c276346ab5ff" />

## Claude Code 상태 연동 (권장)

```bash
python3 scripts/install-claude-hooks.py
```

`~/.claude/settings.json` 에 훅을 합쳐 넣습니다(백업을 먼저 만듭니다). 상태 칩, 알림, 세션 복구의 대화 이어가기가 이 훅으로 정확해집니다. 이미 돌던 Claude 세션은 다시 시작해야 적용됩니다. 매핑은 [docs/claude-hooks.md](docs/claude-hooks.md).

## CLI

pane 안에서는 인자 없이 자기 pane 을 가리킵니다(`AMUX_PANE_ID` 상속).

```bash
amux ls                          # 워크스페이스 > 탭 > pane 목록
amux ws create --tab-name 작업    # 새 워크스페이스 (첫 탭 이름까지)
amux tab new --name 빌드          # 현재 워크스페이스에 새 탭
amux tab focus t-3fa2c1          # 탭 전환
amux tab rename t-3fa2c1 배포     # 탭 이름 변경
amux tab close t-3fa2c1          # 탭과 그 안 pane 전부 닫기
amux split --right               # 현재 pane 을 오른쪽으로 분할
amux send 'git status' --enter   # 텍스트 입력
amux send-keys C-c               # 키 입력 (tmux 식 이름)
amux read-screen p-3fa2c1        # 다른 pane 화면 읽기
amux notify --kind done --title 빌드 --body 완료
```

소켓은 `$XDG_RUNTIME_DIR/amux/amux.sock`(macOS 는 `/tmp/amux-$UID/amux.sock`)이고, NDJSON JSON-RPC 2.0 입니다(`socat` 으로 디버깅).

## 개발

```bash
# 필요: rustup, bun, Tauri 의존성 (리눅스 libwebkit2gtk-4.1-dev 등, macOS Xcode Command Line Tools)
bun install
bun run tauri dev        # 개발 실행 (HMR)
cargo test --workspace   # Rust 테스트
cargo build --release -p amux-cli   # .deb 가 target/release/amux 를 /usr/bin/amux 로 넣으므로 먼저
bun run tauri build      # 릴리스 번들 (리눅스 .deb, macOS .app/.dmg, 윈도우 NSIS)
```

구조: `crates/amux-protocol`(공유 타입), `crates/amux-core`(엔진: PTY, 터미널 상태, 알림, 소켓 서버), `crates/amux-cli`(CLI), `src-tauri`(앱 셸, 채팅, 파일 창), `src`(Svelte 화면).

## 변경 내역

자세한 기록은 [`CHANGELOG.md`](CHANGELOG.md) 에 있습니다.

- **미출시 (main)**: Claude 대화 화면(파일 실시간 보기, 파일 팝업 창, 서브에이전트 칸, 터미널과 세션 이어 쓰기), 닫기 전 확인, 지난 대화 검색, 구독 한도 공용 조회, macOS 지원, 세션 저장과 복구, 한글 입력 중복 수정, 탭 계층, 하단 입력창
- **v0.5.0**: 윈도우 지원, 빈 시작과 직전 명령 칩, 🟣 DONE 상태, Shift+Enter 줄바꿈 수정
- **v0.4.0**: Mission Control, 브로드캐스트, 명령 팔레트, 지금 봐야 할 에이전트
- **v0.3.0**: 테마 38종, 알림과 상태 다듬기
- **v0.2.0**: dock 배지
- **v0.1.0**: 첫 릴리스
