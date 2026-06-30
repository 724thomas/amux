# amux — 윈도우 설치·실행 가이드 (윈도우 PC의 Claude에게 붙여넣는 문서)

> **이 문서는 윈도우 PC에서 실행 중인 Claude Code(또는 Claude)에게 통째로 붙여넣어
> 자동으로 설치·빌드·실행하도록 만든 핸드오프 문서입니다.**
> amux는 원래 Ubuntu용 데스크톱 터미널(AI 코딩 에이전트 병렬 실행기)인데,
> 코드가 윈도우에서도 빌드되도록 이미 크로스플랫폼화되어 있습니다. 당신(윈도우 Claude)의
> 일은 ① 빌드 도구 준비 → ② 빌드 → ③ 실행, 그리고 혹시 컴파일이 막히면 아래 "검증됨 vs
> 미검증" 지도를 보고 고치는 것입니다.

---

## 0. 윈도우 Claude에게 — 먼저 읽을 것

- **스택**: Tauri 2 (Rust 백엔드) + Svelte 5 + xterm.js (프론트엔드). 가짜 터미널(PTY)은
  `portable-pty`가 윈도우 ConPTY로, CLI↔앱 통신은 `interprocess`가 윈도우 named pipe로
  알아서 처리합니다. 즉 **윈도우 고유 코드는 거의 없고**, 필요한 건 주로 빌드 도구입니다.
- **정직한 전제**: 이 코드는 리눅스에서 빌드·테스트가 모두 통과(green)했지만, **윈도우
  컴파일러로는 검증되지 않았습니다.** cfg(windows) 분기(아래 §6)는 리눅스에서 컴파일되지
  않는 부분이라, 만에 하나 에러가 나면 거기일 가능성이 높습니다. 설계 의도와 고치는 법을
  §6·§8에 적어뒀습니다.
- **사람의 승인이 필요한 단계가 있습니다**: 빌드 도구 설치(특히 MSVC)는 관리자 권한(UAC)
  팝업을 띄웁니다. 완전 무인은 아니고, 사용자가 UAC를 한두 번 눌러줘야 합니다.
- **소스 확보**: 이 윈도우 포팅 변경이 들어간 **브랜치/커밋**을 받아야 합니다. 기존
  릴리스나 옛 main을 받으면 윈도우 코드가 없습니다. 사용자에게 "이 변경이 포함된 브랜치"를
  clone 하도록 확인하세요. (아래 §2)

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

이 **윈도우 포팅 변경이 포함된 브랜치**를 clone 하세요. (사용자에게 정확한 저장소 URL과
브랜치명을 확인 — 보통 `https://github.com/724thomas/amux.git`)

```powershell
git clone https://github.com/724thomas/amux.git
cd amux
# 윈도우 포팅 변경이 별도 브랜치에 있다면:  git checkout <브랜치명>
```

> 만약 사용자가 변경분을 아직 push 하지 않았다면, 리눅스 쪽에서 먼저 커밋·push 해야 합니다.

---

## 3. 1단계 — 빠른 실행(dev) 스모크 테스트  ← 여기부터 시작

가장 빠르게 "뜨는지" 확인하는 경로입니다. 무거운 패키징(설치본 만들기)은 §5로 미룹니다.

```powershell
bun install
bun run tauri dev
```

- 처음엔 Rust 의존성 컴파일로 몇 분 걸립니다.
- 성공하면 amux 창이 뜨고, 워크스페이스 하나에 PowerShell pane이 열립니다.
- pane에서 명령을 쳐보고(예: `dir`), 분할(상단 중앙 툴바 ◫/⬓)과 워크스페이스 추가가
  되는지 확인하세요.

빈 창만 뜨면 → WebView2 누락(§1-4). 링커 에러(`link.exe` not found)면 → MSVC 누락(§1-1).

---

## 4. 자가검증 (테스트)

윈도우에서 코드가 제대로 컴파일·동작하는지 스스로 확인하세요.

```powershell
cargo build --workspace
cargo test -p amux-core
```

- `cargo test -p amux-core`에는 **`server::tests::ipc_round_trip`** 가 들어 있습니다. 이건
  서버를 띄우고 클라이언트가 **named pipe로 실제 왕복 통신**을 하는 테스트라, 통과하면
  윈도우 IPC(앱↔CLI 통신)가 런타임에 동작한다는 직접 증거입니다.
- `pane::tests::echo_round_trip` 는 실제 셸(PowerShell)을 띄워 출력을 읽는 테스트입니다.

---

## 5. 2단계 — 패키지 설치본(.exe) 만들기 (선택)

배포용 NSIS 설치본을 만들려면:

```powershell
cargo build --release -p amux-cli   # target\release\amux.exe (CLI) 생성
bun run tauri build                 # tauri.windows.conf.json 덕에 NSIS 설치본(.exe) 생성
```

- 산출물: `src-tauri\target\release\bundle\nsis\*-setup.exe` (또는 `target\release\bundle\nsis\`).
- `tauri build`가 처음 실행될 때 NSIS를 자동으로 내려받습니다(별도 설치 불필요).
- ⚠️ `tauri build` 전에 반드시 `cargo build --release -p amux-cli`를 먼저 — 설정이
  `amux.exe`를 동봉하려 하지 않더라도, CLI는 PATH에 둬야 쓸 수 있습니다(다음 절).

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
PTY, UI)는 그래서 신뢰도가 높습니다. 다만 아래 **cfg(windows) 분기**는 리눅스에서는
컴파일되지 않는 부분이라 윈도우 컴파일러로만 확인됩니다. 에러가 나면 십중팔구 여기입니다.

| 파일 | 윈도우 분기 | 의도 |
|---|---|---|
| `crates/amux-protocol/src/lib.rs` | `default_socket_name()`의 `#[cfg(windows)]` | 소켓 이름을 `amux-<user>.sock`(파이프 이름)으로 |
| `crates/amux-core/src/server.rs` | `local_name()`의 `to_ns_name`, `#[cfg(unix)]` 디렉터리 준비 블록은 윈도우에서 건너뜀 | named pipe 리스너 |
| `crates/amux-cli/src/main.rs` | `local_name()`의 `to_ns_name` | named pipe 클라이언트 |
| `crates/amux-core/src/pane.rs` | `#[cfg(windows)]` 셸 선택(`powershell.exe`, `AMUX_SHELL` 오버라이드) | 윈도우 기본 셸 |

**고칠 때 참고할 API 모양 (버전이 바뀌었으면 해당 크레이트 문서를 확인):**

- `interprocess` 2.x: 이름은 `s.to_ns_name::<GenericNamespaced>()`(윈도우) /
  `s.to_fs_name::<GenericFilePath>()`(유닉스). 서버는
  `ListenerOptions::new().name(name).create_tokio()` → `listener.accept().await`.
  클라이언트(동기)는 `Stream::connect(name)`. tokio 스트림은 `tokio::io::split(stream)`로 분리.
- `portable-pty` 0.9: `native_pty_system().openpty(PtySize{..})`,
  `CommandBuilder::new(shell)`, `pty.slave.spawn_command(cmd)`. `master.process_group_leader()`는
  윈도우에서 `None`을 반환하는 게 정상입니다(이러면 사이드바 cwd/git가 빈 값 — §9).

자가검증은 `cargo test -p amux-core` (특히 `ipc_round_trip` = named pipe 왕복).

---

## 9. 알려진 윈도우 제약 (버그 아님 — 동작 정상)

이건 의도된 v1 한계입니다. "안 되는 것"이 아니라 "아직 안 채운 것"입니다.

1. **사이드바의 cwd(작업 폴더)·리슨 포트가 윈도우에선 비어 보입니다.** 이 정보는 리눅스의
   `/proc` 가상 파일시스템에서 읽는데 윈도우엔 `/proc`가 없어, 코드가 그냥 빈 값을 돌려줍니다
   (우아하게 비활성화). git 브랜치는 `.git`을 직접 읽으므로 윈도우에서도 표시됩니다.
   - **채우고 싶다면 (선택 작업):**
     - cwd: `meta/cwd.rs`에 `#[cfg(windows)]` 분기를 추가하고 `sysinfo` 크레이트의
       `sys.process(Pid).cwd()`로 구현.
     - 리슨 포트: `meta/ports.rs`에 `#[cfg(windows)]` 분기 — `netstat2` 크레이트로 LISTEN
       상태 TCP를 PID와 함께 얻고, `sysinfo`로 자식 프로세스 트리를 모아 그 PID들 소유 포트만
       필터. (리눅스의 inode 매칭 방식과 달리 윈도우 TCP 테이블은 소유 PID를 직접 줍니다.)
2. **데스크톱 토스트 알림이 처음엔 안 뜰 수 있습니다.** notify-rust의 윈도우 토스트는 앱이
   설치되어 AppUserModelID로 등록돼 있어야 뜹니다. NSIS 설치본(§5)으로 설치하면 동작하고,
   `bun run tauri dev`로 띄운 개발 모드에선 조용할 수 있습니다. (사이드바 "지금 봐야 할
   에이전트" 패널과 상태 칩은 토스트와 무관하게 동작합니다.)
3. **"이미 다른 인스턴스가 떠 있음" 가드가 윈도우엔 없습니다.** 유닉스는 stale 소켓을
   정리/감지하지만 named pipe는 다중 인스턴스를 허용합니다. 두 개를 동시에 띄우지 마세요.
   (parity 항목 — v1 차단 요소 아님.)

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

---

## 11. 윈도우 Claude를 위한 한 줄 요약(작업 순서)

1. §1 도구 설치 (특히 §1-1 MSVC — UAC 승인) → 새 셸 열기
2. §2 소스 clone (윈도우 변경 포함 브랜치)
3. `bun install` → `bun run tauri dev` 로 **뜨는지 확인** (§3)
4. `cargo test -p amux-core` 로 IPC·셸 자가검증 (§4)
5. 필요 시 §5 설치본, §6 PATH, §7 hooks
6. 컴파일/동작 문제 → §8(미검증 지도)·§10(문제 해결)

문제가 막히면, 어떤 단계에서 어떤 에러가 났는지 그대로 사용자에게 보고하고, §8의 설계
의도를 근거로 최소 수정만 제안하세요.
