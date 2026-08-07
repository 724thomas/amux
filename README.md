# amux

AI 코딩 에이전트(Claude Code 등)를 **병렬로** 돌리기 위한 Ubuntu 데스크톱 터미널.

<img width="1856" height="1080" alt="image" src="https://github.com/user-attachments/assets/0f635231-9b41-4945-ad60-8dd0414e8d6f" />


- 워크스페이스 × **탭** × 탭 안 분할 pane — 동시 다중 터미널, 개수 제한 없음
- 탭은 화면 한 장이자 **이름을 갖는 단위** — 탭 안에서 자유롭게 분할하고, 브로드캐스트는 그 탭 안에서만 동작
- 에이전트 상태 칩: 🔴 processing / 🟢 processed / 🔵 idle / 🟡 waiting, 그리고 직접 고정하는 🟣 DONE(검토 중)
- 데스크톱 알림 (BEL, OSC 9/777, Claude Code hook) + 사이드바 **지금 봐야 할 에이전트** 우선순위 목록
- 사이드바에 브랜치 · cwd · 리슨 포트(클릭하면 브라우저 오픈) 표시
- **직전 명령 칩** — 각 pane에 마지막으로 보낸 명령을 최대 4줄로 고정 표시 (드래그로 위치 이동 · 터미널 폰트에 연동 · 설정 토글)
- **하단 고정 입력창** — pane 아래에 붙는 프롬프트 작성칸. 긴 출력을 위로 스크롤해 읽는 도중에 다음 프롬프트를 써도 화면이 맨 아래로 튀지 않는다 (Ctrl+Shift+E)
- 모든 조작이 마우스로 가능 (분할·닫기·이름변경·드래그 재배치·테마)
- 키보드 콕핏: ⚡브로드캐스트 · ⌨명령 팔레트 · 🛰Mission Control · 탭 점프(Alt+1–9) · 워크스페이스 점프(Ctrl+Shift+1–9)
- `amux` CLI로 외부 자동화: `ls`, `tab new/close/focus/rename`, `split`, `send`, `read-screen`, `notify` …
- 색 테마 38종 (Tokyo Night 기본 · 다크/라이트)

스택: Tauri 2 (Rust) + Svelte 5 + xterm.js.

## 변경 내역

### 미출시 (main)
- **탭·워크스페이스는 만들기 전에 이름을 묻는다** — `Ctrl+T`(또는 탭바 `+`)를 누르면
  탭바 끝에 입력칸이 생기고, Enter를 눌러야 탭이 만들어진다. 워크스페이스는 태어날
  때 첫 탭을 품고 있으므로 입력이 2단계(`1/2 워크스페이스` → `2/2 첫 탭`)가 됐다.
  이름을 비우고 Enter만 치면 예전처럼 `탭 N` · `워크스페이스 N`으로 자동 명명되므로
  `Ctrl+T` → `Enter` 두 번이면 여전히 한 박자에 빈 탭을 얻는다. 만드는 입구가
  사이드바 `+` · 단축키 · 명령 팔레트 셋인데, 팔레트만 프롬프트를 건너뛰고 곧바로
  만들던 구멍도 함께 막았다 — 이제 셋 다 같은 프롬프트를 연다. CLI에는
  `amux ws create --tab-name` 을 추가했다.
- **🗂 탭 (Ctrl+T / Ctrl+W)** — 워크스페이스와 pane 사이에 **탭** 계층이 생겼다.
  지금까지는 워크스페이스 하나가 분할 트리를 직접 들고 있어서, 터미널을 늘리는 길이
  화면을 쪼개는 것뿐이었다 — 6개를 띄우면 6칸으로 갈라져 하나하나가 좁아졌다. 이제
  **탭 하나가 화면 한 장**이고 그 안에서 기존처럼 분할할 수 있다. 안 보이는 탭도
  프로세스와 스크롤 내용이 그대로 살아 있어(DOM에서 떼지 않고 숨기기만 한다) 에이전트가
  계속 돌아간다. **이름은 탭에만** 붙고 개별 터미널은 이름을 갖지 않는다 — 분할된 칸은
  목록에서 `작업 #1` `작업 #2`처럼 탭 이름 + 순번으로 구분한다. **브로드캐스트
  (Ctrl+Shift+B)도 현재 탭 안으로 좁혔다**: 화면에 없는 터미널에 입력이 날아가는 일이
  구조적으로 불가능해진다. 단축키는 브라우저 감각으로 재배치 — `Ctrl+T` 새 탭 ·
  `Ctrl+W` 탭 닫기 · `Ctrl+Tab`/`Ctrl+Shift+Tab` 탭 전환 · `Alt+1…9` N번째 탭 ·
  `Ctrl+Shift+N` 새 워크스페이스 · `Ctrl+Shift+W` 워크스페이스 닫기. 탭을 만들 때는
  워크스페이스처럼 **제목 입력창이 먼저** 뜨고(비우면 `탭 N`), Enter를 눌러야
  만들어진다. `Ctrl+T`와
  `Ctrl+W`는 Ctrl 단독이라 셸에서 쓰던 키를 가져온다(`Ctrl+W` 단어 삭제 · vim 창 조작,
  `Ctrl+T` fzf 파일 검색)는 점만 알아두면 된다. CLI에도 `amux tab new/close/focus/rename`
  이 생겼고, `amux split`은 의미 그대로(탭 안 분할) 유지된다.
- **단축키 이중 실행 가드를 전체로 확대** — 터미널에 포커스가 있으면 하나의 keydown이
  xterm의 키 핸들러와 window 리스너 양쪽에 전달된다. 지금까지는 이 이중 전달을 토글
  3개(브로드캐스트·하단 입력창·대시보드)에서만 개별 플래그로 막고 있었다. 이제
  `handleKey`가 소비한 이벤트에 표식을 남겨 **모든** 단축키가 정확히 한 번만 실행된다 —
  탭 생성처럼 두 번 실행되면 결과가 두 개 생기는 동작이 늘어나서 개별 가드로는 감당이
  안 된다.
- **⌨ 하단 고정 입력창 (Ctrl+Shift+E)** — pane 맨 아래에 터미널과 분리된 프롬프트
  작성칸을 붙인다. 지금까지는 긴 출력을 위로 스크롤해 읽는 중에 다음 프롬프트를
  타이핑하면 키를 누를 때마다 화면이 맨 아래로 튀었다 — xterm이 "사용자 입력 =
  최신 출력을 봐야 함"으로 보고 강제 스크롤하기 때문(`scrollOnUserInput`).
  입력칸에 쓰는 동안엔 터미널에 아무 키도 들어가지 않으므로 읽던 자리가 그대로
  유지되고, Enter를 누르는 순간에만 텍스트가 PTY로 들어간다. 여러 줄 프롬프트는
  bracketed paste로 감싸 한 번에 전달(줄마다 제출되지 않음), 한글 IME 조합 중
  Enter는 전송이 아니라 글자 확정으로 처리
  - **터미널로 자동 양보** — 한 글자씩 반응하는 UI는 입력창이 대신 받을 수 없으므로,
    빈 입력칸에서 `/` `!` `#` `@` `↑` `↓` `Tab`은 터미널로 그대로 넘긴다(슬래시 명령
    자동완성·옵션 선택). Claude가 턴 도중 권한/선택을 물으면(🟡 waiting) 입력칸이
    노랗게 바뀌고, 쓰던 글이 없을 때만 포커스가 터미널로 넘어간다

### v0.5.0
- **🪟 윈도우 지원** — 원래 Ubuntu 전용이던 앱을 윈도우에서도 빌드·실행되도록
  크로스플랫폼화. IPC를 `interprocess`(유닉스 소켓 ↔ named pipe)로, 기본 셸을 OS별
  분기(윈도우=PowerShell)로, 패키징에 NSIS 설치본(.exe)을 추가. 설치·빌드 안내는
  [`windows_install.md`](windows_install.md) 참고
  - **윈도우 빌드 차단 버그 수정** — `portable-pty`의 `process_group_leader()`가 유닉스
    전용이라 윈도우에선 `amux-core`가 컴파일조차 안 돼 `.exe`가 만들어지지 않던 문제.
    `shell_pid()`를 cfg 게이트해 해결(리눅스 동작 무변경, `cargo check --target
    x86_64-pc-windows-gnu`로 크로스 검증)
- **빈 시작 + 새 워크스페이스 제목 입력 + 직전 명령 칩** — 실행 시 워크스페이스 없이
  시작하고 `+ 새 워크스페이스`로 제목을 정해 만든다. 각 pane엔 마지막으로 보낸 명령을
  최대 4줄로 고정 표시하는 드래그 가능한 칩
- **🟣 DONE 상태 + pane 툴바 ✓ 버튼** — 작업이 끝나 검토 중인 pane을 직접 고정.
  자동 판정되는 나머지 네 상태와 달리 hook·벨·포커스·휴리스틱이 건드리지 못한다
- **Shift+Enter 줄바꿈 수정** — 줄바꿈 대신 프롬프트가 제출되던 버그. 핸들러는
  있었지만 `preventDefault`가 없어 브라우저가 keypress를 한 번 더 쏘고, xterm이
  거기서 Enter를 그대로 PTY에 보내고 있었다 (Alt+Enter는 xterm 자체 경로라 무사)

### v0.4.0
- **🛰 Mission Control 대시보드 (Ctrl+Shift+A)** — 전 워크스페이스의 에이전트를 JARVIS HUD 모달로 한눈에, 노드 클릭 시 그 pane으로 점프
- **⚡ Broadcast Input (Ctrl+Shift+B)** — 워크스페이스 전 pane에 입력 동시 전송 ("한 번 입력, 모든 에이전트")
- **⌨ Command Palette (Ctrl+Shift+P)** — 퍼지 명령 팔레트
- **워크스페이스 탭 점프 (Ctrl+Shift+1…9)** — 사이드바 N번째 워크스페이스로 바로 이동 (위치 기반)
- **지금 봐야 할 에이전트 패널** — 사이드바 하단의 알림 히스토리를 대체. 손길 필요한 pane(🟡 입력 대기 → 🟢 완료·미확인)을 우선순위로 모아 라이브 타이머와 함께 보여주고 클릭 점프
- **Thinking Waveform** — pane 출력 byte-rate에 반응하는 파동 + Arc Reactor 코어 (우상단)
- **Done-Shockwave** — 안 보던 pane이 완료되면 초록 방사 충격파로 시선 유도
- **pane 툴바를 상단 중앙으로** 이동 + 상태/알림 정확도 개선 (turn-lifecycle 기반 processing/processed/waiting 교정, 알림 pane당 1개)

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

[**Releases**](https://github.com/724thomas/amux/releases)에서 OS에 맞는 설치본을 받습니다.

**리눅스 (Ubuntu/Debian)** — `.deb`:

```bash
wget https://github.com/724thomas/amux/releases/download/v0.5.0/amux_0.5.0_amd64.deb
sudo apt install ./amux_0.5.0_amd64.deb
```

- GNOME 앱 목록에 **amux** 아이콘 등록, `amux` CLI는 `/usr/bin/amux`로 설치
- 의존성(webkit2gtk 등)은 apt가 자동 해결
- 빌드 도구 없이 설치 파일 하나로 끝 — 소스 빌드는 아래 [개발](#개발) 참고

빈 화면이 뜨면 (WebKitGTK + Wayland DMABUF 이슈):

```bash
WEBKIT_DISABLE_DMABUF_RENDERER=1 amux-app
```

**윈도우 (10/11)** — NSIS 설치본(`.exe`):

Releases에서 윈도우 설치본(`amux_0.5.0_x64-setup.exe` 형태)을 받아 실행하면 설치됩니다.
`amux` CLI(`amux.exe`)를 PATH에 넣고 Claude 상태 연동 hook을 켜는 방법 등 자세한 안내는
[`windows_install.md`](windows_install.md)를 참고하세요. (윈도우 바이너리는 리눅스에서 만들 수
없어, `v*` 태그를 푸시하면 GitHub Actions의 windows-latest 러너가 자동으로 빌드해 같은
릴리스에 첨부합니다.)

## 사용법

### 화면 구성

왼쪽 **사이드바**(워크스페이스 → 탭 목록 / 지금 봐야 할 에이전트 / 테마·폰트) +
오른쪽 **터미널 영역**(위에 탭바, 아래에 그 탭의 분할 pane). 사이드바 폭은 경계선 드래그로 조절.

**탭**은 화면 한 장입니다. 한 워크스페이스는 탭을 여러 개 갖고 한 번에 하나만 화면에
보이며, 안 보이는 탭의 에이전트도 계속 돌아갑니다(스크롤 내용도 그대로 유지). 이름은
탭에만 있고 — 개별 터미널은 이름이 없습니다 — 탭 안에서 분할한 각 칸은 목록에서
`작업 #1` `작업 #2`처럼 탭 이름에 순번이 붙어 구분됩니다. 탭바에서 클릭해 전환,
더블클릭해 이름 변경, 드래그해 순서 변경, `×`나 휠 클릭으로 닫습니다.

탭을 만들 때는 워크스페이스와 마찬가지로 **제목 입력창이 먼저** 뜹니다 — `+`를
누르거나 `Ctrl+T`를 치면 탭바 끝에 입력칸이 생기고, 이름을 적고 Enter를 눌러야
비로소 탭이 만들어집니다. 이름 없이 그냥 Enter를 치면 `탭 N`으로 자동 명명되므로
`Ctrl+T` → `Enter` 두 번만에 빈 탭을 얻을 수도 있습니다. Esc는 취소입니다.

amux는 **워크스페이스 없이 빈 상태로 시작**합니다 — `+ 새 워크스페이스`(또는
`Ctrl+Shift+N`)를 누르면 **제목 입력창**이 떠서(포커스 자동 이동) 이름을 묻습니다.
워크스페이스는 태어날 때 이미 첫 탭을 하나 품고 있으므로 입력은 **두 단계**입니다 —
`1/2 워크스페이스`에서 이름을 넣고 Enter, 이어서 `2/2 첫 탭`에서 그 탭 이름을 넣고
Enter를 치면 그때 만들어집니다(2단계에서는 방금 정한 워크스페이스 이름이 옆에
칩으로 표시됩니다). 각 단계에서 이름을 비우면 `워크스페이스 N` · `탭 1`로 자동
명명되고, Esc는 언제든 취소입니다.

각 pane 안에는 **직전 명령 칩**이 마지막으로 보낸 명령을 최대 4줄로 고정 표시합니다.
`⠿`를 잡고 원하는 곳으로 드래그해 옮길 수 있고(위치 저장), 터미널 폰트 크기에 맞춰
텍스트가 함께 커집니다. 사이드바 하단 폰트 아래 토글로 켜고 끕니다.

각 pane 맨 아래에는 **하단 고정 입력창**이 붙습니다. 여기에 프롬프트를 쓰면 터미널
스크롤이 전혀 움직이지 않으므로, 긴 출력을 위로 올려 읽으면서 다음 지시를 작성할 수
있습니다. `Enter` 전송 · `Shift+Enter` 줄바꿈 · `Ctrl+Enter` 제출 없이 입력만 ·
`Esc` 터미널로 복귀 · `Ctrl+Shift+E` 입력창↔터미널 오가기. 세 줄 이상이 되면 터미널
위로 겹쳐 자라고(터미널 크기는 그대로), 보낸 뒤에는 결과를 볼 수 있게 맨 아래로
스크롤합니다. 사이드바 하단 `하단 입력창` 토글로 끌 수 있습니다.

문장을 조립해 한 번에 보내는 입력창과 달리, **한 글자씩 반응하는 UI**(슬래시 명령
자동완성 · 옵션 메뉴 · 권한 질문)는 터미널이 직접 받아야 합니다. 그래서 입력칸이
**비어 있을 때** 다음 키는 터미널로 그대로 넘어갑니다 — `/` `!` `#` `@` (Claude Code의
명령·bash·메모리·파일 멘션 접두사)와 `↑` `↓` `Tab`. `/`를 누르면 곧바로 터미널에서
자동완성이 열리고 이어서 타이핑하면 됩니다. 또 Claude가 **턴 도중 권한/선택을 물어오면**
(사이드바 🟡 waiting) 입력칸이 노랗게 바뀌며 "터미널이 선택을 기다립니다"로 표시되고,
쓰던 글이 없을 때는 포커스가 자동으로 터미널로 넘어갑니다. 작성 중인 글이 있으면
절대 뺏지 않습니다.

사이드바 하단 **지금 봐야 할 에이전트** 패널은 손길 필요한 pane(🟡 입력 대기 →
🟢 완료·미확인)을 우선순위로 모아 라이브 타이머와 함께 보여줍니다 (행 클릭 → 점프).

### 마우스 (모든 조작 가능)

| 하고 싶은 것 | 방법 |
|---|---|
| 새 워크스페이스 | 사이드바 `+ 새 워크스페이스` → 이름 Enter → 첫 탭 이름 Enter |
| 새 탭 | 탭바 오른쪽 `+` → 제목 입력 후 Enter (비우면 `탭 N`) |
| 탭 전환 / 이름 변경 / 순서 변경 / 닫기 | 탭 클릭 / 더블클릭 / 드래그 / `×`·휠 클릭 |
| 워크스페이스·탭 전환 | 사이드바 항목 클릭 (키보드 포커스 자동 이동) |
| 탭 안 분할 | pane에 마우스 올리면 상단 중앙 툴바 ◫(오른쪽) ⬓(아래) |
| pane 재배치 | 툴바 ⠿를 드래그 → **같은 탭 안** 다른 pane의 상/하/좌/우에 드롭 |
| 크기 조절 | 분할선 드래그, 더블클릭하면 50:50 |
| 이름 변경 / 닫기 | 사이드바 항목 우클릭 |
| 복사 | 텍스트 드래그하면 자동 복사 (우클릭 메뉴도 있음) |
| 붙여넣기 | 휠 클릭 또는 Ctrl+V |
| 폰트 크기 | Ctrl+휠 또는 사이드바 하단 Aa − ＋ (직전 명령 칩도 함께 스케일) |
| 직전 명령 칩 이동 | 칩 왼쪽 `⠿` 잡고 드래그 (위치 저장) |
| 직전 명령 칩 켜기/끄기 | 사이드바 하단 폰트 아래 `직전 명령 표시` 토글 |
| 하단 입력창 켜기/끄기 | 사이드바 하단 `하단 입력창` 토글 (명령 팔레트에도 있음) |
| 하단 입력창으로 전송 | 입력칸에 쓰고 Enter (또는 오른쪽 `⏎` 버튼) |
| 테마 | 사이드바 하단 테마 드롭다운 |
| 서버 열기 | 사이드바 포트 칩(`:5173`) 클릭 → 브라우저 |
| URL 열기 | 터미널 안 링크 Ctrl+클릭 |

<img width="1846" height="1072" alt="image" src="https://github.com/user-attachments/assets/77ee872f-81ee-46f8-bb4b-8614405b5a4e" />


### 키보드

| 키 | 동작 |
|---|---|
| Ctrl+T | 새 탭 (제목 입력창) |
| Ctrl+W | 탭 닫기 |
| Ctrl+Tab / Ctrl+Shift+Tab | 다음 / 이전 탭 |
| Alt+1…9 | N번째 탭으로 바로 이동 |
| Ctrl+Shift+N | 새 워크스페이스 (이름 → 첫 탭 이름, 2단계 입력창) |
| Ctrl+Shift+W | 워크스페이스 닫기 (그 안의 탭·pane 전부) |
| Ctrl+Shift+D / S | 탭 안에서 오른쪽 / 아래 분할 |
| Alt+방향키 | 탭 안 pane 간 이동 |
| Ctrl+PgUp / PgDn | 워크스페이스 전환 |
| Ctrl+Shift+1…9 | N번째 워크스페이스로 바로 이동 |
| Ctrl+Shift+B | 브로드캐스트 (**현재 탭**의 pane 동시 입력) |
| Ctrl+Shift+P | 명령 팔레트 |
| Ctrl+Shift+A | Mission Control 대시보드 |
| Ctrl+Shift+E | 하단 입력창 ↔ 터미널 포커스 이동 (꺼져 있으면 켜면서 이동) |
| Shift+Enter | 줄바꿈 (Claude Code 입력창 포함 · 하단 입력창에서도 동일) |
| Shift+방향키 | 커서 기준 텍스트 선택 |
| Ctrl+C / X / V | 복사(선택 시) / 잘라내기(선택 시) / 붙여넣기 |
| Ctrl+Shift+F | 스크롤백 검색 |
| Ctrl+= / − / 0 | 폰트 크기 |

### 에이전트 상태 칩

사이드바의 탭마다, 그리고 탭바의 탭 색 점으로 상태가 항상 표시됩니다. 탭 안에
터미널이 여럿이면 **그중 가장 급한 상태**가 그 탭의 상태로 올라옵니다
(🟡 waiting → 🟢 processed → 🔴 processing → 🔵 idle → 🟣 DONE 순):

| 칩 | 의미 |
|---|---|
| 🔴 processing… | 작업 진행 중 |
| 🟢 processed | 작업 완료, 아직 안 봄 |
| 🔵 idle | 한가함 (완료 확인됨) |
| 🟡 waiting | 입력 대기 (예: Claude 권한 질문) |
| 🟣 DONE | 작업이 끝나 **검토 중** — 사용자가 직접 고정 |

앞의 네 개는 amux가 자동으로 판정합니다. Claude Code는 hook 연동 시
정확하게 동작하고(아래 참고), 일반 명령은 출력 휴리스틱으로 판정됩니다.

**🟣 DONE만 예외로, 직접 고정하는 상태입니다.** pane에 마우스를 올리면 뜨는
툴바의 맨 왼쪽 **✓** 버튼을 누르면 그 pane이 DONE으로 고정됩니다. 고정된
동안에는 hook·벨·포커스·휴리스틱 무엇도 상태를 바꾸지 못하므로, 에이전트가
계속 뭔가를 출력해도 "검토 중" 표시가 유지됩니다. ✓를 다시 누르면 고정이
풀리고 idle로 돌아가면서 자동 판정이 재개됩니다.

> 상태는 저장되지 않습니다 — amux를 재시작하면 DONE 고정도 함께 사라집니다.

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
amux ls                          # 워크스페이스 → 탭 → pane 목록
amux ws create --tab-name 작업    # 새 워크스페이스 (첫 탭 이름까지 지정)
amux tab new --name 빌드          # 현재 워크스페이스에 새 탭
amux tab focus t-3fa2c1          # 탭 전환
amux tab rename t-3fa2c1 배포     # 탭 이름 변경 (그 안 모든 pane의 표시 이름)
amux tab close t-3fa2c1          # 탭과 그 안 pane 전부 닫기
amux split --right               # 현재 pane을 같은 탭 안에서 오른쪽 분할
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
