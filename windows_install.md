# amux — 윈도우 설치·실행 가이드 (윈도우 PC의 Claude에게 붙여넣는 문서)

> **이 문서는 윈도우 PC에서 실행 중인 Claude Code(또는 Claude)에게 통째로 붙여넣어
> 자동으로 설치·빌드·실행하도록 만든 핸드오프 문서입니다.**
> amux는 원래 Ubuntu용 데스크톱 터미널(AI 코딩 에이전트 병렬 실행기)인데,
> 코드가 윈도우에서도 빌드되도록 이미 크로스플랫폼화되어 있습니다. 당신(윈도우 Claude)의
> 일은 ① 빌드 도구 준비 → ② 빌드 → ③ 실행, 그리고 혹시 컴파일이 막히면 아래 "검증됨 vs
> 미검증" 지도를 보고 고치는 것입니다.
>
> **다른 OS 문서**: macOS는 [`macos_install.md`](macos_install.md), Ubuntu는
> [`ubuntu_install.md`](ubuntu_install.md). (macOS는 이미 실검증되어 cwd/포트까지
> 채워져 있어, §9-1의 윈도우 cwd/포트 미구현을 채울 때 좋은 참고가 됩니다.)

---

## 0. 윈도우 Claude에게 — 먼저 읽을 것

- **스택**: Tauri 2 (Rust 백엔드) + Svelte 5 + xterm.js (프론트엔드). 가짜 터미널(PTY)은
  `portable-pty`가 윈도우 ConPTY로, CLI↔앱 통신은 `interprocess`가 윈도우 named pipe로
  알아서 처리합니다. 즉 **윈도우 고유 코드는 많지 않고**, 필요한 건 주로 빌드 도구입니다.
- **정직한 전제**: 이 코드는 **실제 윈도우 PC에서 빌드·테스트·실행까지 확인됐습니다**
  (Windows 11 / MSVC / PowerShell 5.1). 사이드바 메타데이터·프로세스 트리·폰 사이드카의
  윈도우 분기도 실기 테스트가 붙어 있습니다(§4). 그래도 환경은 저마다 다르니, 막히면
  §8의 설계 의도와 §10의 증상별 표를 보세요.
- **사람의 승인이 필요한 단계가 있습니다**: 빌드 도구 설치(특히 MSVC)는 관리자 권한(UAC)
  팝업을 띄웁니다. 완전 무인은 아니고, 사용자가 UAC를 한두 번 눌러줘야 합니다.
- **소스 확보**: 윈도우 포팅은 **`main`에 병합**되어 있습니다. `main`을 clone 하면 됩니다.
  (아래 §2)

---

## 1. 사전 도구 설치

PowerShell을 **관리자 권한으로** 열고 진행하세요. (`winget`은 Windows 10 1709+/11에 기본 탑재)

### 1-1. (가장 중요) MSVC C++ 빌드 도구 — 이게 없으면 Rust 빌드가 링크 단계에서 깨집니다

Rust-on-Windows에서 **가장 흔한 실패 원인**입니다. rustup은 MSVC *타깃* 툴체인은 깔지만
링커(`link.exe`)와 Windows SDK는 깔지 않습니다. 그래서 별도로 설치해야 합니다.

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e `
  --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

- "Desktop development with C++" 워크로드(컴포넌트 id 대략 `Microsoft.VisualStudio.Workload.VCTools`)를
  포함해야 합니다. 위 컴포넌트 id가 안 먹으면 Visual Studio Installer를 GUI로 열어
  **"C++를 사용한 데스크톱 개발"** 워크로드를 체크해 설치하세요.
- **UAC 승인 필요**, 설치에 수 분~십수 분 걸리고 용량이 큽니다.
- 설치 후 PowerShell을 **새로 열어** PATH를 갱신하세요.

### 1-2. Rust (rustup)

```powershell
winget install --id Rustlang.Rustup -e
```

설치 후 새 PowerShell에서 확인:

```powershell
rustc --version   # 기본 타깃이 x86_64-pc-windows-msvc 여야 함
```

### 1-3. Bun (프론트엔드 빌드 도구)

```powershell
powershell -c "irm bun.sh/install.ps1 | iex"
```

설치 후 **새 PowerShell**을 열어 PATH 반영. 확인:

```powershell
bun --version
```

### 1-4. WebView2 런타임 (Tauri 앱이 화면을 그리는 엔진)

Windows 11에는 기본 포함, Windows 10도 대개 Edge와 함께 있습니다. 없거나 빈 창이 뜨면:

```powershell
winget install --id Microsoft.EdgeWebView2Runtime -e
```

### 1-5. Git (소스 받기용, 이미 있으면 생략)

```powershell
winget install --id Git.Git -e
```

---

## 2. 소스 받기

윈도우 포팅은 `main`에 있습니다. 그대로 clone 하세요.

```powershell
git clone https://github.com/724thomas/amux.git
cd amux
```

---

## 3. 1단계 — 빠른 실행(dev) 스모크 테스트  ← 여기부터 시작

가장 빠르게 "뜨는지" 확인하는 경로입니다. 무거운 패키징(설치본 만들기)은 §5로 미룹니다.

```powershell
bun install
bun run tauri dev
```

- 처음엔 Rust 의존성 컴파일로 몇 분 걸립니다.
- 성공하면 amux 창이 뜹니다. **지금은 실행하자마자 워크스페이스가 열리지
  않습니다("빈 시작"으로 바뀌었음).** 왼쪽 사이드바의 **`+ 새 워크스페이스`** 버튼을
  누르고 제목을 입력한 뒤 Enter를 치면, 그때 PowerShell pane 하나가 열립니다.
  (예전 문서는 실행 즉시 pane이 열린다고 안내했는데, 지금은 사용자가 첫 워크스페이스를
  직접 만드는 방식입니다.)
- 워크스페이스가 열리면 pane에서 명령을 쳐보고(예: `dir`), 분할(상단 중앙 툴바 ◫/⬓)이
  되는지 확인하세요.

**빈-창 진단 주의:** 사이드바와 `+ 새 워크스페이스` 버튼까지 다 안 보이고 **새하얀 창**만
뜨면 → WebView2 누락(§1-4). 반면 **사이드바는 떴는데 가운데만 비어 있는 것은 정상**입니다
(위의 `+ 새 워크스페이스`로 시작하세요). 링커 에러(`link.exe` not found)면 → MSVC 누락(§1-1).

---

## 4. 자가검증 (테스트)

윈도우에서 코드가 제대로 컴파일·동작하는지 스스로 확인하세요.

```powershell
cargo build --workspace
cargo test --workspace --exclude amux-app
```

전부 실제 OS를 건드리는 테스트라, 통과하면 그 기능이 런타임에 동작한다는 직접 증거입니다.

- **`server::tests::ipc_round_trip`** — 서버를 띄우고 클라이언트가 **named pipe로 실제 왕복
  통신**을 합니다. 윈도우 IPC(앱↔CLI 통신)가 되는지.
- **`pane::tests::echo_round_trip`** — 실제 셸(PowerShell)을 띄워 출력을 읽습니다. ConPTY가
  시작할 때 던지는 커서 위치 질의(`ESC[6n`)에 답해 줘야 셸이 출력을 흘리기 시작하는데,
  실제 앱에서는 xterm.js가 하는 그 일을 테스트에서는 `Pane::answer_cursor_queries()` 가
  대신합니다. 제출 키가 LF가 아니라 **CR** 인 것도 여기서 드러납니다(PSReadLine).
- **`win_proc::tests::*`** — 프로세스 트리(자식 등장 / 형제 중 최신 / 자식 없음 / 스냅샷
  공유·만료). 사이드바 메타데이터와 상태 칩이 전부 여기에 얹혀 있습니다.
- **`meta::cwd::windows_smoke::*`** — 자기 자신과 **다른 프로세스**의 작업 폴더 읽기.
- **`meta::ports::windows_smoke::*`** — 자기 포트와 **자식이 쥔 포트** 탐지.
- **`engine::tests::a_windows_pane_reports_the_directory_its_command_runs_in`** — 끝에서 끝까지.
  pane을 띄우고 `Set-Location` 으로 옮긴 뒤 명령을 돌려, 사이드바에 뜰 폴더가 맞게 잡히고
  **명령이 끝난 뒤에도 유지되는지** 봅니다 (§9-1의 PowerShell 함정).
- **`tls::tests::*`** (amux-phone) — 발급자가 보증하는 주소 범위, 그리고 실제 발급자를 만들어
  사설망 주소로 인증서를 찍는 것까지.

---

## 5. 2단계 — 패키지 설치본(.exe) 만들기 (선택)

배포용 NSIS 설치본을 만들려면:

```powershell
cargo build --release -p amux-cli -p amux-phone   # amux.exe (CLI) + amux-phone.exe
bun run tauri build                               # tauri.windows.conf.json 덕에 NSIS 설치본(.exe) 생성
```

- 산출물: `src-tauri\target\release\bundle\nsis\*-setup.exe` (또는 `target\release\bundle\nsis\`).
- `tauri build`가 처음 실행될 때 NSIS를 자동으로 내려받습니다(별도 설치 불필요).
- ⚠️ `tauri build` 전에 반드시 `cargo build --release -p amux-cli`를 먼저 — 설정이
  `amux.exe`를 동봉하려 하지 않더라도, CLI는 PATH에 둬야 쓸 수 있습니다(다음 절).
- ⚠️ **이미 amux가 떠 있으면 빌드가 링크 단계에서 깨집니다.** 윈도우는 실행 중인
  `.exe`를 덮어쓰지 못합니다. 빌드 전에 앱을 닫으세요
  (`Get-Process amux-app -ErrorAction SilentlyContinue | Stop-Process`).
- `amux-phone`은 설치본에 들어가지 않습니다 — 폰 기능은 §12 참고.

---

## 6. `amux` CLI를 PATH에 (중요 — 안 하면 hooks·CLI가 안 됨)

NSIS 설치본은 **`amux.exe` CLI를 자동으로 PATH에 넣지 않습니다.** pane 안에서 `amux ls`,
`amux split` 같은 명령과 **Claude 상태 연동 hooks**(§7)가 동작하려면 `amux.exe`가 PATH에
있어야 합니다.

가장 간단한 방법 — 사용자 PATH에 빌드 폴더를 추가(새 셸부터 적용):

```powershell
# release 빌드를 했다면:
$amuxDir = "$PWD\target\release"
[Environment]::SetEnvironmentVariable(
  "Path", [Environment]::GetEnvironmentVariable("Path","User") + ";$amuxDir", "User")
```

또는 `amux.exe`를 이미 PATH에 있는 폴더(예: `%USERPROFILE%\bin`)에 복사하세요. 확인:

```powershell
# 새 PowerShell에서
amux --version
```

---

## 7. Claude Code 상태 연동 hooks (선택, 권장)

pane 안에서 도는 Claude Code가 상태 칩(🔴/🟢/🟡)을 정확히 표시하도록 hook을 설치합니다.
**먼저 §6으로 `amux.exe`가 PATH에 있어야 합니다.**

```powershell
python scripts\install-claude-hooks.py
```

- `~/.claude/settings.json`(윈도우는 `C:\Users\<당신>\.claude\settings.json`)에 hook을
  병합합니다(기존 설정 백업). 스크립트는 윈도우/리눅스를 자동 감지합니다.
- hook 명령은 단순히 `amux notify ...` 형태입니다 — CLI가 amux 앱에 연결 못 하면 조용히
  종료(exit 0)하도록 만들어져 있어, pane 밖이나 앱이 꺼져 있어도 Claude 세션을 깨지
  않습니다. (그래서 셸별 `2>nul` 같은 군더더기가 없습니다.)
- Python이 없으면 `winget install --id Python.Python.3.12 -e`.

---

## 8. 검증됨 vs 미검증 — 컴파일이 막히면 여기를 보세요

리눅스에서 **빌드·테스트가 모두 통과**했고, 양쪽 OS가 공유하는 코드(IPC 전송, 프로토콜,
PTY, UI)는 그래서 신뢰도가 높습니다. 아래 **cfg(windows) 분기**는 리눅스 *네이티브* 빌드엔
안 들어가지만, 이제 **`cargo check --target x86_64-pc-windows-gnu`로 크로스 컴파일 검증**했습니다
— 코어 크레이트(`amux-core`·`amux-protocol`·`amux-cli`)는 **테스트 포함 그린**입니다. 남은
미검증은 `src-tauri`(윈도우 리소스 컴파일러·WebView2가 필요해 리눅스 크로스 불가 — CI의
windows job이 실검증)와 **모든 런타임 동작**뿐입니다. 컴파일 에러가 나면 십중팔구 아래입니다.

| 파일 | 윈도우 분기 | 의도 |
|---|---|---|
| `crates/amux-protocol/src/lib.rs` | `default_socket_name()`의 `#[cfg(windows)]` | 소켓 이름을 `amux-<user>.sock`(파이프 이름)으로 |
| `crates/amux-core/src/server.rs` | `local_name()`의 `to_ns_name`, `#[cfg(unix)]` 디렉터리 준비 블록은 윈도우에서 건너뜀 | named pipe 리스너 |
| `crates/amux-cli/src/main.rs` | `local_name()`의 `to_ns_name` | named pipe 클라이언트 |
| `crates/amux-core/src/pane.rs` | `#[cfg(windows)]` 셸 선택(`powershell.exe`, `AMUX_SHELL` 오버라이드) | 윈도우 기본 셸 |
| `crates/amux-core/src/win_proc.rs` | 파일 전체가 `#[cfg(windows)]` | pane 셸의 프로세스 트리 — ConPTY 에 없는 포그라운드 프로세스 그룹을 대신합니다. ToolHelp 스냅샷 하나를 0.5초 동안 공유 |
| `crates/amux-core/src/meta/mod.rs` | `foreground_pid()` · `resolve_cwd()` 의 `#[cfg(windows)]` | 어느 PID 에게 cwd 를 물을지 고르는 자리 |
| `crates/amux-core/src/meta/cwd.rs` | `#[cfg(windows)]` 분기 (`sysinfo`) | 프로세스의 작업 폴더는 PEB 에 있고 오프셋이 비공개라 sysinfo 에 위임 |
| `crates/amux-core/src/meta/ports.rs` | `#[cfg(windows)] mod imp` (`GetExtendedTcpTable`) | LISTEN 포트. 윈도우 TCP 테이블은 소유 PID 를 직접 줍니다 |
| `crates/amux-phone/src/tls.rs` | `config_base()` 의 `%APPDATA%` 폴백 | 윈도우엔 `$HOME`/XDG 가 없어, 없으면 CA 개인키가 실행 폴더에 떨어집니다 |

**고칠 때 참고할 API 모양 (버전이 바뀌었으면 해당 크레이트 문서를 확인):**

- `interprocess` 2.x: 이름은 `s.to_ns_name::<GenericNamespaced>()`(윈도우) /
  `s.to_fs_name::<GenericFilePath>()`(유닉스). 서버는
  `ListenerOptions::new().name(name).create_tokio()` → `listener.accept().await`.
  클라이언트(동기)는 `Stream::connect(name)`. tokio 스트림은 `tokio::io::split(stream)`로 분리.
- `portable-pty` 0.9: `native_pty_system().openpty(PtySize{..})`,
  `CommandBuilder::new(shell)`, `pty.slave.spawn_command(cmd)`.
  ⚠️ **함정(이미 고쳤음):** `master.process_group_leader()`는 portable-pty에서 **`#[cfg(unix)]`
  전용**이라 윈도우엔 그 메서드가 **아예 없습니다.** 그냥 호출하면 "None을 반환"하는 게 아니라
  `no method named process_group_leader`로 **컴파일이 막혀 `amux-core`가 통째로 빌드 실패**하고,
  그러면 `bunx tauri build`(→ `amux-app` → `amux-core`)도 실패해 **.exe가 아예 안 만들어집니다.**
  그래서 `pane.rs::shell_pid()`를 `#[cfg(unix)]`로 감싸 **윈도우에선 `None`**, Unix 분기는
  글자 그대로 유지하도록 했습니다. 윈도우 쪽 호출자는 이걸 쓰지 않고 `win_proc` 의
  프로세스 트리에 같은 질문을 합니다 (§9-1).

자가검증은 `cargo test -p amux-core` (특히 `ipc_round_trip` = named pipe 왕복).

---

## 9. 알려진 윈도우 제약 (버그 아님 — 동작 정상)

이건 의도된 v1 한계입니다. "안 되는 것"이 아니라 "아직 안 채운 것"입니다.

1. **사이드바의 cwd·git 브랜치·리슨 포트는 채워집니다. 단, 프롬프트가 놀고 있는 동안의
   `cd`는 다음 명령을 칠 때까지 반영되지 않습니다.**

   유닉스는 PTY에게 "지금 이 pane의 포그라운드가 누구냐"를 물으면 되지만(`tcgetpgrp`),
   ConPTY에는 프로세스 그룹이 없습니다. 그래서 윈도우는 **셸의 자손 프로세스**로 대신
   답합니다(`win_proc.rs`) — 프롬프트에서 친 명령은 셸의 자식으로 뜨니까요.

   여기에 PowerShell 특유의 함정이 하나 있고, 이게 위 제약의 이유입니다.
   **`Set-Location`은 PowerShell의 provider 위치만 옮기고 프로세스 작업 폴더는 그대로
   둡니다**(런스페이스가 공유하는 값이라 스레드 안전하지 않아 일부러 그럽니다). 즉 셸
   자신에게 물으면 영원히 "amux가 처음 띄운 폴더"가 나옵니다. 대신 PowerShell은 자기가
   띄우는 명령에는 provider 위치를 작업 폴더로 물려주므로, **돌고 있는 명령이 정답을
   압니다.** 명령이 끝난 뒤에는 그 답을 그대로 유지합니다 — 사용자가 `cd`를 치는 건
   볼 수 없지만, 이미 틀린 걸 아는 폴더로 되돌아가는 것보다 낫기 때문입니다.

   실제로 겪는 모습: pane을 열고 `cd 프로젝트` 만 치면 사이드바는 아직 홈 폴더를
   가리킵니다. 거기서 `claude`(또는 아무 외부 명령)를 한 번 띄우면 그때 맞는 폴더로
   바뀌고, 그 뒤로는 유지됩니다. amux의 용도가 에이전트 병렬 실행이라 실사용에서는
   거의 걸리지 않습니다.

   - cwd: `meta/cwd.rs`의 `#[cfg(windows)]` — 작업 폴더는 PEB에 있고 오프셋이 문서화돼
     있지 않아, 이미 그걸 읽는(32/64비트 차이까지 처리하는) `sysinfo`에 맡깁니다.
   - 리슨 포트: `meta/ports.rs`의 `#[cfg(windows)] mod imp` — `GetExtendedTcpTable`의
     LISTEN 목록을 프로세스 트리로 거릅니다. 리눅스의 inode 매칭과 달리 윈도우 TCP
     테이블은 소유 PID를 직접 주므로 fd를 훑을 필요가 없습니다.
   - git 브랜치는 `.git`을 직접 읽으므로 cwd만 맞으면 따라옵니다.
2. **데스크톱 토스트 알림이 처음엔 안 뜰 수 있습니다.** notify-rust의 윈도우 토스트는 앱이
   설치되어 AppUserModelID로 등록돼 있어야 뜹니다. NSIS 설치본(§5)으로 설치하면 동작하고,
   `bun run tauri dev`로 띄운 개발 모드에선 조용할 수 있습니다. (사이드바 "지금 봐야 할
   에이전트" 패널과 상태 칩은 토스트와 무관하게 동작합니다.)
3. **"이미 다른 인스턴스가 떠 있음" 가드가 윈도우엔 없습니다.** 유닉스는 stale 소켓을
   정리/감지하지만 named pipe는 다중 인스턴스를 허용합니다. 두 개를 동시에 띄우지 마세요.
   (parity 항목 — v1 차단 요소 아님.)
4. **상태 칩(🔴 작업 중 / 🟢 완료 / 🟡 입력 대기)의 "지금 뭔가 돌고 있나" 판별은 윈도우에서도
   동작합니다.** amux는 pane 출력이 조용해지는 패턴으로 상태를 추측하는 휴리스틱(heuristic,
   경험적 추정)을 쓰는데, 그 재료인 "포그라운드에 앱이 도는가"를 윈도우에서는 셸의 자식
   프로세스 유무로 봅니다(`Pane::app_running`). 한 가지 사각지대: **PowerShell 안에서만 도는
   cmdlet**(`Start-Sleep`, `1..9 | %{...}` 등)은 자식을 만들지 않아 잡히지 않습니다. 외부
   실행 파일(claude, node, npm, git…)은 모두 잡히므로 실사용에는 영향이 없습니다.
   그래도 **§7의 Claude 상태 hook을 설치하는 편이 정확합니다** — hook은 Claude가 상태를 직접
   통보하므로 추정에 의존하지 않습니다.
5. **kitty 키보드 모드 자동 정리도 이제 윈도우에서 돌아갑니다(단, 실사용 검증은 아직).**
   Claude Code 같은 앱이 켜는 특수 키보드 모드(kitty keyboard protocol)를, 앱이 **비정상
   종료**하면 amux가 "셸이 다시 포그라운드로 돌아온 것"을 감지해 꺼줍니다(`meta/mod.rs`의
   `compute()`). 이 감지는 `fg_pid == child_pid()` 비교인데, 윈도우 `foreground_pid()`가
   자식이 없을 때 셸 자신을 돌려주므로 유닉스와 같은 뜻이 됐습니다. 애초에 영향 범위가 좁아
   (앱이 **정상 종료**하면 스스로 모드를 끕니다) 크래시를 일부러 재현해 확인하지는 않았습니다.

---

## 10. 문제 해결 (자주 나는 에러)

| 증상 | 원인 / 해결 |
|---|---|
| `error: linker `link.exe` not found` / `link.exe` 관련 | MSVC 빌드 도구 누락 → §1-1 |
| 앱 창이 **하얗게/빈 화면** | WebView2 런타임 누락 → §1-4 |
| `bun: command not found` | bun PATH 미반영 → 새 PowerShell 열기 (§1-3) |
| `cargo test`의 `ipc_round_trip` 실패 | `interprocess` API가 버전에 따라 바뀌었을 수 있음 → §8의 API 모양 확인 |
| `amux: command not found` (pane 안에서) | `amux.exe`가 PATH에 없음 → §6 |
| hook이 안 먹음 | ① `amux.exe` PATH(§6) ② Claude Code 세션 재시작 필요 |
| `tauri build` 중 NSIS 관련 실패 | 첫 실행은 NSIS를 내려받습니다(네트워크 필요). 재시도 |
| 빌드가 링크 단계에서 "Access is denied" / 파일 사용 중 | amux가 떠 있습니다. 닫고 다시 → §5 |
| `.ps1` 실행했더니 한글이 깨져 나옴 | BOM 없이 저장된 스크립트. UTF-8 **BOM 포함**으로 저장 → §12 |
| 폰이 `https://<주소>:8000` 에 못 붙음 | ① 방화벽 규칙 없음 ② 발급자 미설치 → §12 |

---

## 12. 폰에서 보기 (amux-phone, 선택)

자리를 비운 사이 폰으로 pane을 들여다보는 사이드카입니다. 리눅스·macOS는
`scripts/phone.sh`, **윈도우는 `scripts\phone.ps1`** 로 켭니다 (같은 일을 합니다).

```powershell
cargo build --release -p amux-phone

scripts\phone.ps1 -Setup   # 맨 처음 한 번 — 폰에 발급자(인증서)를 설치
scripts\phone.ps1          # 평소 — 켜고 나갔다가 돌아와서 Ctrl+C
scripts\phone.ps1 -Pair    # 새 폰을 등록할 때 (QR + 6자리 코드)
```

스크립트가 알아서 하는 것:

- **주소**를 라우팅 표에서 매번 찾습니다. DHCP 임대는 바뀌니까요.
- **사설 대역(10.x · 172.16~31.x · 192.168.x)이 아니면 아예 뜨지 않습니다.** 공인 주소로
  바인딩하면 인터넷 전체에 포트를 여는 셈이라서입니다. 사내망(10.x)이 아닌 사설 대역이면
  "같은 공유기에 붙은 기기는 닿을 수 있다"고 경고합니다 — 집 공유기와 카페 와이파이는
  주소만 봐서는 구분되지 않습니다.
- **윈도우 방화벽**에 해당 포트를 여는 규칙이 없으면 알려줍니다. 리눅스의 ufw 절과 같은
  자리지만 더 자주 걸립니다 — 윈도우 방화벽은 기본으로 켜져 들어오는 연결을 막습니다.
  ```powershell
  # 관리자 PowerShell. 지금 붙어 있는 망(Private)에서만 열립니다.
  New-NetFirewallRule -DisplayName 'amux phone' -Direction Inbound `
    -Action Allow -Protocol TCP -LocalPort 8000 -Profile Private
  ```

알아둘 것:

- 발급자(CA)와 설정은 **`%APPDATA%\amux\`** 에 들어갑니다 (유닉스의 `~/.config/amux/` 자리).
- 발급자는 사설 대역만 보증하도록 제한돼 있고 **도메인 이름은 전면 배제**합니다. 즉 이
  발급자를 신뢰하는 폰이 속을 수 있는 최대치는 "사설망의 어떤 기기"이지 웹사이트가 아닙니다.
- **v0.5.0 이전에 만든 발급자가 있으면** 10.x 만 보증합니다. 192.168 주소로 켜려 하면 시작
  단계에서 막고 안내합니다 — `%APPDATA%\amux\ca\` 의 `ca.crt`·`ca.key` 를 지우고
  `-Setup` 으로 다시 만든 뒤 폰에 재설치하세요.
- **`.ps1` 은 UTF-8 BOM 으로 저장해야 합니다.** Windows PowerShell 5.1은 BOM이 없으면
  스크립트를 ANSI 코드페이지로 읽어 한글 안내문이 전부 깨집니다. 편집기가 BOM을 떼지
  않도록 주의하세요.

---

## 13. 윈도우 Claude를 위한 한 줄 요약(작업 순서)

1. §1 도구 설치 (특히 §1-1 MSVC — UAC 승인) → 새 셸 열기
2. §2 소스 clone (윈도우 변경 포함 브랜치)
3. `bun install` → `bun run tauri dev` 로 **뜨는지 확인** (§3)
4. `cargo test -p amux-core` 로 IPC·셸 자가검증 (§4)
5. 필요 시 §5 설치본, §6 PATH, §7 hooks, §12 폰
6. 컴파일/동작 문제 → §8(미검증 지도)·§10(문제 해결)

문제가 막히면, 어떤 단계에서 어떤 에러가 났는지 그대로 사용자에게 보고하고, §8의 설계
의도를 근거로 최소 수정만 제안하세요.
