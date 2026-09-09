# amux — macOS 설치·실행 가이드 (macOS의 Claude에게 붙여넣는 문서)

> **이 문서는 macOS에서 실행 중인 Claude Code(또는 Claude)에게 통째로 붙여넣어
> 자동으로 설치·빌드·실행하도록 만든 핸드오프 문서입니다.**
> amux는 원래 Ubuntu용 데스크톱 터미널(AI 코딩 에이전트 병렬 실행기)인데, macOS 이식이
> 끝나 **Apple Silicon(arm64)에서 빌드·실행·end-to-end 동작까지 검증**되었습니다. 당신
> (macOS Claude)의 일은 ① 빌드 도구 준비 → ② 빌드·설치 → ③ 실행 확인입니다.

---

## 0. macOS Claude에게 — 먼저 읽을 것

- **스택**: Tauri 2 (Rust 백엔드) + Svelte 5 + xterm.js (프론트엔드). 가짜 터미널(PTY)은
  `portable-pty`가, CLI↔앱 통신은 `interprocess`가 처리합니다. macOS는 유닉스 계열이라
  리눅스와 **거의 같은 코드 경로**를 씁니다 (Unix 도메인 소켓, `$SHELL -l` 로그인 셸 등).
- **정직한 전제 (검증 상태)**: 이 이식은 **Apple Silicon에서 실제로 검증**되었습니다 —
  `.app`/CLI 빌드, `amux-core` 테스트 24개 통과(PTY 왕복·세션 복원 포함), macOS 전용
  cwd/포트 런타임 스모크 테스트 통과, 그리고 앱 실행 → 소켓 기동 → CLI로 워크스페이스·탭·
  pane(zsh) 생성까지 end-to-end 확인. Intel Mac(x86_64)에서도 동일 코드가 컴파일되지만
  네이티브 실검증은 Apple Silicon 기준입니다.
- **사람의 승인이 필요한 단계**: 빌드 도구 설치(Xcode Command Line Tools, Homebrew)와,
  서명 안 된 앱 첫 실행 시 Gatekeeper 우회(우클릭 → 열기)는 사용자 조작이 필요합니다.
  대부분은 sudo 없이 됩니다(설치 위치를 사용자 홈으로 잡을 경우).
- **소스 확보**: macOS 지원은 **`main`에 병합**되어 있습니다. `main`을 clone 하면 됩니다.

---

## 1. 사전 도구 설치

### 1-1. Xcode Command Line Tools (컴파일러·링커·libclang)

```bash
xcode-select --install     # 이미 있으면 "already installed" 라고 나옴
xcode-select -p            # /Library/Developer/CommandLineTools 등이 나오면 OK
```

> libproc(아래 §8)가 빌드 시 `bindgen`으로 헤더를 읽어 바인딩을 만드는데, 그때 CLT의
> libclang이 필요합니다. CLT가 없으면 libproc 컴파일이 막힙니다.

### 1-2. Homebrew (없으면)

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

설치 후 안내되는 `eval "$(/opt/homebrew/bin/brew shellenv)"`(Apple Silicon) 줄을 셸
프로필에 반영하세요.

### 1-3. Rust (rustup)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
rustc --version    # 예: rustc 1.9x (aarch64-apple-darwin)
```

### 1-4. Bun (프론트엔드 빌드 도구)

```bash
brew install oven-sh/bun/bun
bun --version
```

---

## 2. 소스 받기

```bash
git clone https://github.com/724thomas/amux.git
cd amux
# macOS 지원은 main 에 있습니다. 특정 브랜치를 쓰라는 안내가 없으면 main 그대로.
```

---

## 3. 1단계 — 빠른 실행(dev) 스모크 테스트  ← 여기부터 시작

```bash
bun install
bun run tauri dev
```

- 처음엔 Rust 의존성 컴파일로 몇 분 걸립니다.
- 성공하면 amux 창이 뜹니다. **실행하자마자 워크스페이스가 열리지 않습니다("빈 시작").**
  왼쪽 사이드바의 **`+ 새 워크스페이스`** 버튼을 눌러 제목을 입력하면 그때 zsh pane이
  하나 열립니다. (이전 세션이 있으면 화면 가운데 "지난 세션이 남아 있습니다" 복구 카드가
  뜹니다 — 눌러서 복구.)
- **이건 버그가 아니라 의도된 동작**입니다(리눅스와 동일). 자동 복원을 하지 않는 이유는
  셸 여러 개가 한꺼번에 되살아나는 것을 막기 위해서입니다.

**빈-창 진단:** 사이드바까지 안 보이고 새하얀 창만 뜨면 프론트엔드 로드 실패 — 콘솔 로그를
확인하세요. **사이드바는 떴는데 가운데만 비어 있는 것은 정상**입니다(`+ 새 워크스페이스`로 시작).

---

## 4. 자가검증 (테스트)

```bash
source "$HOME/.cargo/env"
cargo test -p amux-core
```

- `pane::tests::echo_round_trip` — 실제 셸(zsh)을 띄워 명령 출력을 읽는 테스트. 통과하면
  PTY가 런타임에 동작한다는 직접 증거입니다.
- `server::tests::ipc_round_trip` — Unix 소켓으로 앱↔CLI 왕복 통신을 검증.
- `engine::tests::session_round_trip_*` — 세션 저장/복구.
- `meta::cwd::macos_smoke` / `meta::ports::macos_smoke` — **macOS 전용** libproc 경로가
  자기 프로세스의 cwd와 LISTEN 포트를 실제로 읽어오는지 확인(§8).

전부 통과해야 정상입니다(현재 기준 24개 그린).

---

## 5. 2단계 — 앱 설치 (`.app` + CLI)

가장 간단한 경로 — 준비된 스크립트 하나로 빌드·설치를 끝냅니다:

```bash
scripts/install-macos.sh
```

이 스크립트가 하는 일:
1. `bun install`
2. `cargo build --release -p amux-cli` (CLI)
3. `bun run tauri build --bundles app` (**`.app`만** 빌드 — 이유는 §9-1)
4. `amux.app` → `/Applications/amux.app` 복사
5. `amux` CLI → PATH에 설치 (기본 `/usr/local/bin`, `AMUX_BIN_DIR`로 변경 가능)
6. quarantine 속성 제거 안내

옵션:
```bash
AMUX_BIN_DIR="$HOME/.local/bin" scripts/install-macos.sh   # sudo 없이 홈에 CLI 설치
scripts/install-macos.sh --build-only                       # 빌드만, 설치 안 함
```

> **sudo 없이 설치하려면**: `/Applications`는 관리자 계정이면 sudo 없이 쓸 수 있고, CLI는
> `AMUX_BIN_DIR=$HOME/.local/bin`(또는 `/opt/homebrew/bin`)처럼 PATH에 이미 있는
> 사용자 쓰기 가능 폴더를 지정하면 sudo가 필요 없습니다.

산출물(수동 확인용):
- 앱: `target/release/bundle/macos/amux.app`
- CLI: `target/release/amux`

---

## 6. 실행 & `amux` CLI

- **앱 실행**: Spotlight에서 `amux`, 또는 `open -a amux`.
- **첫 실행(서명 안 됨)**: `amux.app`을 **우클릭 → 열기**, 또는:
  ```bash
  xattr -dr com.apple.quarantine /Applications/amux.app
  ```
- **CLI**: pane은 `zsh -l`(로그인 셸)로 뜨므로, 앱을 GUI로 켜도 pane 안에서는 사용자
  프로필의 PATH가 로드됩니다. `amux`를 그 PATH에 설치했다면(§5) pane에서 바로 `amux ls`
  등이 동작합니다.
  ```bash
  amux --version                  # 0.5.0
  amux ls                         # 워크스페이스 → 탭 → pane
  amux ws create --tab-name 작업
  amux send 'git status' --enter --pane <pane-id>
  ```

> 소켓 경로: macOS엔 `$XDG_RUNTIME_DIR`가 없어 `/tmp/amux-$UID/amux.sock`로 폴백합니다
> (`socat - UNIX-CONNECT:/tmp/amux-$UID/amux.sock`로 디버깅 가능).

---

## 7. Claude Code 상태 연동 hooks (선택, 권장)

```bash
python3 scripts/install-claude-hooks.py
```

- `~/.claude/settings.json`에 hook을 병합 설치(기존 설정 백업). 크로스플랫폼 스크립트입니다.
- hook은 `amux notify ...` 형태이며, CLI가 앱에 연결 못 하면 조용히 종료(exit 0)하도록
  만들어져 pane 밖이나 앱이 꺼져 있어도 Claude 세션을 깨지 않습니다.
- **`amux` CLI가 hook 실행 환경의 PATH에 있어야** 합니다(§6). 이미 돌던 Claude 세션은
  재시작해야 hook 설정을 다시 읽습니다.

---

## 8. macOS 이식의 핵심 — cwd/포트를 libproc로

리눅스는 pane의 현재 디렉터리·git 브랜치·리슨 포트를 `/proc`에서 읽습니다. macOS엔 `/proc`가
없어, 그 부분만 **libproc**로 새로 구현했습니다. 컴파일/동작 문제가 나면 십중팔구 여기입니다.

| 파일 | macOS 분기 | 의도 |
|---|---|---|
| `crates/amux-core/src/meta/cwd.rs` | `#[cfg(target_os="macos")]` — `proc_pidinfo(PROC_PIDVNODEPATHINFO)`로 `pvi_cdir.vip_path` | pane cwd |
| `crates/amux-core/src/meta/ports.rs` | `#[cfg(target_os="macos")]` — `ProcFilter::ByParentProcess`로 자식 BFS → 소켓 fd → TCP LISTEN 로컬 포트 | 리슨 포트 |
| `crates/amux-core/src/meta/git.rs` | (분기 없음) `.git/HEAD` 직접 읽기 — cwd만 있으면 동작 | git 브랜치 |
| `crates/amux-core/src/notify.rs` | `.urgency()` 호출을 `#[cfg(not(target_os="macos"))]`로 게이트 | 알림(우선순위 무시) |
| `crates/amux-core/Cargo.toml` | `[target.'cfg(target_os="macos")'.dependencies]`에 `libproc`·`libc` | |
| `src-tauri/tauri.macos.conf.json` | `bundle.targets = ["app","dmg"]` | 번들 타겟 |
| `scripts/install-macos.sh` | 빌드+설치 | |

**고칠 때 참고 (버전이 바뀌었으면 크레이트 문서 확인):**
- `libproc` 0.14: `pidcwd()`는 macOS에서 미구현 스텁이라 쓰지 말고, cwd는 `libc`의
  `proc_pidinfo` + `proc_vnodepathinfo`를 직접 호출합니다. `vip_path`는 libc에서
  `[[c_char; 32]; 32]`(구 rustc 우회용 2D 배열)이라 평탄화 포인터 캐스트가 필요합니다.
- 포트: `libproc::processes::pids_by_type(ProcFilter::ByParentProcess{ppid})`로 자식을 찾고
  (`/proc/<pid>/children`이 없으므로), `listpidinfo::<ListFDs>` → 소켓 fd만
  `pidfdinfo::<SocketFDInfo>` → `SocketInfoKind::Tcp` && `TcpSIState::Listen` 필터.
  `insi_lport`는 네트워크 바이트 오더라 `u16::from_be(x as u16)`.

자가검증: `cargo test -p amux-core meta::` (macOS 스모크 테스트가 cwd·포트를 실제로 읽음).

---

## 9. 알려진 macOS 제약 / 확인 필요 (버그 아님)

1. **`.dmg` 로컬 빌드는 실패할 수 있습니다.** tauri의 dmg 번들러가 Finder/AppleScript로
   창을 꾸미는 단계라 헤드리스/자동화 환경에서 자주 실패합니다. **로컬 설치엔 `.app`만
   있으면 되므로** 설치 스크립트는 `--bundles app`으로 빌드합니다. 배포용 `.dmg`가
   필요하면 GUI 데스크톱 세션이나 macOS CI 러너에서 `bun run tauri build`를 돌리세요.
2. **데스크톱 알림은 "뜨는지" 확인이 필요합니다.** 알림 디스패치는 에러 없이 실행되지만,
   macOS는 **서명 안 된 앱**에 알림 권한 프롬프트를 안 띄우고 조용히 억제할 수 있습니다.
   그리고 우선순위 단계(Critical/Normal/Low)는 macOS엔 없어 무시됩니다(코드에서 게이트).
   → 사용자에게 알림 센터에 실제로 뜨는지 확인 요청. 안 뜨면 시스템 설정 → 알림에서 amux
   허용, 또는 앱 서명을 검토.
3. **Dock 배지 카운트**(미확인 완료 개수)는 Tauri `set_badge_count`의 macOS Dock 경로로
   동작하리라 보이나 별도 실검증 안 됨(위험 낮음).
4. **폰 사이드카 `scripts/phone.sh`는 macOS에서 안 됩니다.** `ip route`·`ss`·`ufw`·`10.*`
   사내망 가드가 전부 리눅스 전용입니다. `amux-phone` **바이너리 자체는 크로스플랫폼**
   (`cargo run -p amux-phone`으로 직접 구동 가능)이지만 런처 스크립트가 리눅스 전용이라,
   폰 기능을 쓰려면 스크립트를 macOS용(`ipconfig getifaddr`, `route -n get`, `lsof
   -iTCP -sTCP:LISTEN`, `pfctl`)으로 포팅해야 합니다.

---

## 10. 문제 해결 (자주 나는 에러)

| 증상 | 원인 / 해결 |
|---|---|
| libproc/bindgen 컴파일 실패 (`libclang`) | Xcode CLT 누락 → §1-1 (`xcode-select --install`) |
| `cargo: command not found` | `source "$HOME/.cargo/env"` (또는 새 셸) → §1-3 |
| `bun: command not found` | `brew install oven-sh/bun/bun` → §1-4 |
| `"amux"을(를) 열 수 없습니다 (개발자 확인 불가)` | 서명 안 된 앱 → 우클릭 → 열기, 또는 `xattr -dr com.apple.quarantine /Applications/amux.app` |
| 앱은 떴는데 **터미널이 안 보임** | 정상(빈 시작) → `+ 새 워크스페이스` 누르거나 복구 카드 수락 (§3) |
| pane에서 `amux: command not found` | CLI가 PATH에 없음 → §5의 `AMUX_BIN_DIR`로 재설치 |
| `tauri build`가 dmg 단계에서 실패 | 정상 — 설치엔 `.app`만 필요, `--bundles app` 사용 (§9-1) |
| 알림이 안 뜸 | §9-2 (서명/권한) |

---

## 11. macOS Claude를 위한 한 줄 요약(작업 순서)

1. §1 도구 설치 (Xcode CLT · Homebrew · rustup · bun) → `source ~/.cargo/env`
2. §2 `git clone` (main)
3. `bun install` → `bun run tauri dev` 로 **뜨는지 확인** (§3)
4. `cargo test -p amux-core` 로 PTY·IPC·세션·macOS 메타 자가검증 (§4)
5. `scripts/install-macos.sh` 로 `.app`+CLI 설치 (§5), 필요 시 §7 hooks
6. 문제 → §8(libproc 지도)·§9(제약)·§10(문제 해결)

문제가 막히면, 어떤 단계에서 어떤 에러가 났는지 그대로 사용자에게 보고하고, §8의 설계
의도를 근거로 최소 수정만 제안하세요.
