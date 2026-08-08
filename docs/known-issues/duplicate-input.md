# 타이핑 중 이미 쓴 글이 다시 들어오는 문제 (중복 입력)

> **상태: 해결됨 (2026-08-08).** 원인은 xterm.js 의 IME 처리 결함이었고, 한글처럼
> 글자를 제자리에서 바꿔 가며 완성하는 입력에서만 터진다. 자세한 것은 10장.
>
> 이 문서는 **고치는 과정 자체를 기억하기 위한 것**이다. 같은 증상으로 여러 번 손을
> 댔는데 매번 처음부터 다시 추측하고 있어서, "이미 해 본 것"과 "배제된 것"을 갈라 둔다.
> 비슷한 증상이 다시 나오면 **3장(이미 시도한 수정)과 3-6장(배제된 가설)을 먼저 읽고,
> 거기 있는 것은 다시 하지 말 것.**
>
> 마지막 갱신: 2026-08-08 · 기준 커밋: `7168aa5`

---

## 1. 증상

사용자 보고(2026-08-08, 원문): **"타이핑을 하다가, 갑자기 자동으로 작성한게 다시 붙여넣어지는 상황"**

즉 사용자가 amux 안의 pane(터미널 칸)에서 글을 치고 있는 도중에, **이미 쳤던 내용이
한 덩어리로 다시 입력에 들어온다.** 사용자가 붙여넣기를 누른 적은 없다.

같은 계열로 과거에 여러 번 수정이 들어갔고, 그때마다 "고쳐졌다"고 판단했지만 증상이
다시 나타났다. 이것이 이 문서를 만드는 이유다 — **매번 다른 원인의 다른 중복이었을
가능성이 높다.** "중복 입력"이라는 한 단어에 최소 다섯 가지 서로 다른 메커니즘이 섞여
있으므로, 재현 조건을 먼저 좁히지 않으면 또 같은 자리를 돌게 된다.

### 아직 확정되지 않은 것 (매우 중요)

아래는 코드만 봐서는 알 수 없고 **사용자 관찰이 있어야만** 좁혀진다. 5장에 질문
형태로 정리해 뒀다. 이게 채워지기 전까지 6장의 가설 순위는 확정할 수 없다.

- 되풀이되는 위치가 **터미널 자체**인지 **하단 입력창(composer)** 인지
- **한글 조합 중**에만 생기는지, 영문 타이핑에서도 생기는지
- 되돌아오는 것이 **방금 친 한 줄**인지 **이전에 보낸 블록 전체**인지

---

## 2. 배경 — 글자가 PTY 로 들어가는 경로 전부

이 문제를 이해하려면 amux 에서 "입력"이 하나의 길이 아니라는 것을 먼저 알아야 한다.
`src/lib/Terminal.svelte`(xterm.js 터미널 하나를 감싸고 PTY 와 연결하는 컴포넌트)에는
**PTY 로 바이트를 흘려보내는 자리가 일곱 군데** 있다. 중복 입력은 이 중 두 경로가
같은 글자를 각각 한 번씩 보낼 때 생긴다.

```ts
// src/lib/Terminal.svelte — writePane(=PTY 로 바이트 쓰기) 호출 지점 전부
sendDraft()      : line 230  // 하단 입력창의 초안을 Enter 로 전송 (여러 줄이면 bracketed paste 로 감싼다)
sendDraft()      : line 234  // 위와 같은 내용을 broadcast 대상 pane 들에도 복사
sendDraft()      : line 238  // 여러 줄 전송 시, 40ms 뒤에 Enter 만 따로 한 번 더
handOff()        : line 264  // 입력창이 비었을 때 "/" "!" "#" "@" ↑↓ Tab 을 터미널로 넘김
cutSelection()   : line 426  // Ctrl+X — 선택 길이만큼 Backspace/Delete 를 보내 실제로 지움
Shift+Enter 분기 : line 469  // 줄바꿈 (kitty 모드면 CSI-u, 아니면 ESC+CR)
Escape 분기      : line 481  // kitty 모드의 plain Esc → CSI 27u
term.onData()    : line 722  // ★ 평소 타이핑의 정규 경로 — xterm 이 만든 바이트를 그대로 전달
```

여기에 더해, **키 하나가 두 번 전달되는 구조**가 원래부터 있다.
`src/lib/keymap.ts`(앱 단축키를 해석하는 모듈)의 주석이 그 사실을 기록하고 있다 —
키보드가 터미널 안에 있을 때 keydown 하나는 xterm 의 `attachCustomKeyEventHandler` 에
한 번, 그리고 window 리스너에 또 한 번, 총 두 번 도달한다.

---

## 3. 이미 시도한 수정 — **여기 있는 것은 다시 하지 말 것**

각 항목은 "무엇을 고쳤나 · 어떤 메커니즘이었나 · **왜 지금 증상을 설명하지 못하나**"
순서다. 마지막 줄이 핵심이다. 그게 없으면 다음 사람이 또 이 자리를 판다.

### 3-1. `830a1da` — Ctrl+V 이중 붙여넣기 (2026-06-12)

**메커니즘**: amux 의 자체 붙여넣기 경로(Rust 클립보드 플러그인 → `term.paste()`)와
WebKit 의 네이티브 paste 이벤트가 **둘 다** xterm 의 숨은 textarea 에 도달해서, 한 번의
Ctrl+V 가 두 번 붙었다.

**한 일**: Ctrl+V / Ctrl+Shift+V 분기에 `e.preventDefault()` 를 넣어 브라우저 기본 동작을
끊고, 터미널 호스트 요소에 `onpastecapture` 를 달아 네이티브 paste 이벤트를 통째로 삼켰다.

```svelte
<!-- src/lib/Terminal.svelte — 지금도 살아 있음 (line 880) -->
<div class="terminal-host"
  onpastecapture={(e) => {   // 네이티브 paste 경로를 캡처 단계에서 잡아
    e.preventDefault();       // 기본 동작을 막고
    e.stopPropagation();      // 아래로 내려가지도 못하게 한다
  }}
></div>
```

**지금 증상을 설명하지 못하는 이유**: 이 방어는 **아직 코드에 그대로 있다**(제거되거나
리팩터링에 휩쓸리지 않았음을 2026-08-08 에 확인). 그리고 이 경로는 **사용자가 Ctrl+V 를
눌러야만** 작동한다. 지금 증상에는 붙여넣기 조작이 없다.

### 3-2. `67fae64` — Shift+Enter 가 줄바꿈 대신 제출되던 버그 (2026-07-27)

**메커니즘**: xterm 의 `_keyDown` 은 커스텀 핸들러가 `false` 를 반환하면 `preventDefault`
없이 곧장 return 한다. 그래서 브라우저가 이어서 keypress 를 쏘고, xterm 의 `_keyPress` 가
charCode 13 을 그대로 PTY 에 보낸다(`\r`). 결국 amux 가 보낸 줄바꿈 **직후에 Enter 가 한 번
더** 들어가 프롬프트가 제출돼 버렸다.

**한 일**: Shift+Enter 분기에서 `e.preventDefault()` 를 명시적으로 호출해 keypress 자체를 막았다.

**지금 증상을 설명하지 못하는 이유**: 이건 **키 하나(Enter)가 두 번** 들어가는 문제였다.
지금 증상은 **글 덩어리가 통째로** 다시 들어오는 것이라 규모가 다르다. 다만 "핸들러가
false 를 반환해도 xterm 이 preventDefault 를 대신 해주지 않는다"는 **교훈은 여전히 유효**하니,
새 키 분기를 추가할 때는 반드시 `preventDefault` 를 같이 넣을 것.

### 3-3. `handOff()` 의 포커스 이동 지연 — "//" 중복 (시기 미상, 코드 주석에만 기록)

**메커니즘**: 하단 입력창이 비어 있을 때 `/` 를 누르면 그 글자를 PTY 로 보내고 포커스를
터미널로 옮긴다. 그런데 keydown **도중에** xterm 의 숨은 textarea 로 포커스를 옮기면,
이어지는 keypress 를 xterm 이 받아 같은 글자를 PTY 에 한 번 더 보낸다 → `//`.

**한 일**: 포커스 이동을 `setTimeout(..., 0)` 으로 그 키 이벤트가 끝난 뒤로 미뤘다.

```ts
// src/lib/Terminal.svelte — handOff() (line 263)
function handOff(data: string) {
  void writePane(pane, data);          // 누른 글자를 PTY 로 먼저 보내고
  // …broadcast 복사 생략…
  setTimeout(() => term?.focus(), 0);  // 포커스 이동은 이 키 이벤트가 끝난 뒤로 미룬다
}
```

**지금 증상을 설명하지 못하는 이유**: 역시 **글자 하나**의 중복이고, `/ ! # @ ↑ ↓ Tab`
일곱 키에서만 발생한다. 다만 이 항목은 **"포커스를 옮기면 입력이 중복될 수 있다"는
전례**라서 6장 가설 A 와 직결된다.

### 3-4. `keymap.ts` 의 이중 전달 가드

**메커니즘**: 2장에서 말한 대로 keydown 하나가 두 경로로 두 번 도달한다. 그대로 두면
단축키가 두 번 실행된다(예: Ctrl+Shift+D 를 한 번 눌렀는데 두 번 분할).

**한 일**: 이벤트 객체 자체에 처리 표시를 찍어 두 번째 도달을 무시한다.

```ts
// src/lib/keymap.ts — handleKey()
export function handleKey(e: KeyboardEvent): boolean {
  if (e.type !== "keydown") return false;
  const stamped = e as KeyboardEvent & { __amuxHandled?: boolean };
  if (stamped.__amuxHandled) return true;   // 이미 처리한 이벤트면 여기서 끝
  const consumed = dispatch(e);
  if (consumed) stamped.__amuxHandled = true;  // 처리했으면 표시를 찍는다
  return consumed;
}
```

**지금 증상을 설명하지 못하는 이유**: 이 가드는 **단축키(Ctrl/Alt 조합)** 에만 해당한다.
일반 글자 입력은 `dispatch()` 가 `false` 를 반환하므로 표시가 찍히지 않고, 애초에 이
경로로 PTY 에 써지지도 않는다.

### 3-5. `7c39317` — 에코 게이팅 (입력 직후 출력을 작업 출력으로 세지 않음)

**한 일**: 입력 직후 1.5초 안에 온 출력은 에코/프롬프트 다시 그리기로 간주해 상태 판정에서
제외했다.

**지금 증상과 무관한 이유**: 이건 **상태 표시(점 색깔)** 계산 문제였고, PTY 로 들어가는
입력 자체는 건드리지 않는다. 중복 입력과 관계없다. (다시 이 자리를 보지 않도록 기록해 둔다.)

---

## 3-6. 배제된 가설 — **다시 파지 말 것**

조사 중에 그럴듯해 보였지만 **코드로 배제된** 것들이다. 근거를 남겨 두니 다음 사람은
같은 길을 다시 가지 말 것.

### 배제 ①: "구독할 때 과거 출력을 다시 흘려보내서 중복된다"

`crates/amux-core/src/pane.rs` 의 `set_sink()` 는 실제로 과거 출력 버퍼(`tail`)를
다시 흘려보낸다. 언뜻 보면 "화면에 옛 내용이 통째로 다시 나타난다"의 원인처럼 보인다.

```rust
// crates/amux-core/src/pane.rs — set_sink()
pub fn set_sink(&self, sink: OutputSink) {
    let mut sink_slot = self.sink.lock();
    {
        let tail = self.tail.lock();
        if !tail.is_empty() {
            sink(&tail);          // 뒤늦게 붙은 구독자에게 최근 출력을 다시 보낸다
        }
    }
    *sink_slot = Some(sink);
}
```

**배제 이유**: 이 replay 는 `src/lib/Terminal.svelte` 의 `onMount` 에서만 일어나고,
그 시점의 xterm 은 **방금 만들어진 빈 터미널**이다(`term.dispose()` 로 옛 것은 파괴됨).
빈 화면을 최근 출력으로 채우는 것이므로 **중복이 아니라 복원**이다. 컴포넌트가 다시
마운트되더라도 결과는 같다.

### 배제 ②: "단축키 해석기가 일반 타이핑을 가로챈다"

`src/lib/keymap.ts` 의 `dispatch()` 는 **모든 분기가 `e.ctrlKey` 또는 `e.altKey` 를
요구**한다(유일한 예외는 대시보드가 열려 있을 때의 Escape). 따라서 수식키 없는 평범한
글자 입력은 이 경로로 들어오지 않는다.

### 배제 ③: "붙여넣기가 저절로 실행된다"

`pasteClipboard()` 를 부르는 자리는 네 곳뿐이고(Ctrl+V · Ctrl+Shift+V · 컨텍스트 메뉴 ·
가운데 클릭) **전부 사용자의 물리적 조작이 필요**하다. 저절로 발동하는 경로는 없다.

---

## 4. 코드에서 새로 확인된 사실 (2026-08-08 조사)

### 4-1. ★ 터미널 입력 경로에는 IME 조합 가드가 전혀 없다

이번 조사에서 나온 가장 쓸모 있는 사실이다. 한글은 자모를 조립해 한 글자를 만드는
**조합(composition)** 과정을 거치는데, 이 조합이 진행 중인지 여부를 브라우저는
`KeyboardEvent.isComposing` 으로 알려 준다. 조합 중의 키 이벤트를 일반 키처럼 처리하면
같은 글자가 두 번 들어가거나 조합 문자열 전체가 다시 흘러나온다.

그런데 `src/lib/Terminal.svelte` 전체에서 이 가드가 **단 한 군데**, 하단 입력창에만 있다.

```ts
// src/lib/Terminal.svelte — composerKey() (line 275). 하단 입력창에는 가드가 있다.
function composerKey(e: KeyboardEvent) {
  // 한글/일본어 IME 조합 중의 Enter는 "글자 확정"이지 전송이 아니다.
  if (e.isComposing || e.keyCode === 229) return;   // ← 여기, 유일한 조합 가드
  …
}
```

반면 **터미널 쪽 입력 경로 세 곳에는 아무 가드가 없다.**

```ts
// src/lib/Terminal.svelte — term.attachCustomKeyEventHandler() (line 453) : 가드 없음
// src/lib/Terminal.svelte — term.onData()                     (line 719) : 가드 없음
// src/lib/keymap.ts       — handleKey()                                  : 가드 없음
```

이 비대칭은 **"입력창에서는 멀쩡한데 터미널에서만 이상하다"** 또는 **"한글 칠 때만
이상하다"** 는 관찰과 곧바로 연결된다. 5장의 질문 (b)(c)가 이걸 가른다.

### 4-2. 선택하면 시스템 클립보드가 조용히 덮어써진다

`src/lib/Terminal.svelte` 는 리눅스 터미널 관례를 따라 **선택만 해도 자동 복사**한다.

```ts
// src/lib/Terminal.svelte — onMount 안 (line 545)
term.onSelectionChange(() => {                  // 선택이 바뀔 때마다
  clearTimeout(selectionTimer);
  selectionTimer = setTimeout(() => {           // 150ms 잠잠해지면
    if (term.hasSelection()) void writeText(term.getSelection());  // 시스템 클립보드에 쓴다
  }, 150);
});
```

주의할 점이 둘이다.

첫째, **`term.select()` 를 코드가 부를 때도 이게 똑같이 발동한다.** Shift+방향키로 터미널
키보드 선택을 확장하는 `extendKeyboardSelection()`(line 407)이 `term.select(...)` 를 부르고,
컨텍스트 메뉴의 "모두 선택"이 `term.selectAll()`(line 843)을 부른다. **스크롤백 검색
(Ctrl+Shift+F)의 `search.findNext()` 도 xterm 내부에서 일치 구간을 선택할 가능성이 있다**
(미확인 — 확인 방법은 6장 가설 C).

둘째, 리눅스의 관례적인 "선택 = 복사"는 **PRIMARY 선택 버퍼**에 들어가지 CLIPBOARD 를
건드리지 않는데, 이 구현은 Tauri 의 `writeText()` 로 **CLIPBOARD 를 덮어쓴다.** 즉
사용자가 Ctrl+C 로 복사해 둔 내용이 터미널에서 뭔가 선택되는 순간 날아간다.

### 4-3. 붙여넣기는 사용자 조작 없이는 발동하지 않는다 (확인됨)

`pasteClipboard()` 를 부르는 자리는 정확히 네 곳이고, **전부 사용자의 물리적 조작이 필요**하다.

```
line 507 : Ctrl+Shift+V
line 537 : Ctrl+V
line 840 : 컨텍스트 메뉴의 "붙여넣기"
line 877 : 가운데 버튼 클릭 (onauxclick, 리눅스 터미널 관례)
```

**그래서 "4-2 의 자동 복사 + 붙여넣기" 조합은 "자동으로"라는 사용자 표현과 맞지 않는다.**
가운데 클릭이 끼어야만 성립하므로 6장에서 2순위 가설로 내려 뒀다. 단, 노트북 터치패드의
**가운데 클릭 에뮬레이션**(세 손가락 탭 등)이 켜져 있으면 사용자가 "클릭했다"고 인식하지
못한 채 발동할 수 있으므로 완전히 배제하지는 않는다.

### 4-4. 사용자 조작 없이 포커스를 옮기는 코드가 있다

이게 "자동으로"와 맞는 유일한 메커니즘이다. `src/lib/Terminal.svelte` 에는 **엔진이 보내는
상태 변화만으로** 키보드 포커스를 터미널로 옮기는 effect 가 있다.

```ts
// src/lib/Terminal.svelte — (line 314). 사용자가 아무것도 안 해도 발동한다.
const waitingInput = $derived(paneInfo(pane)?.status === "waiting");
let prevWaiting = false;
$effect(() => {
  const w = waitingInput;                       // 엔진이 이 pane 을 waiting 으로 바꾸면
  if (w && !prevWaiting && untrack(() => composerFocused && draft === "")) {
    term?.focus();                              // 포커스를 터미널로 가져온다
  }
  prevWaiting = w;
});
```

포커스를 옮기는 자리는 이것 말고도 여럿이다 — `handOff()` 의 `setTimeout(() => term?.focus(), 0)`
(line 272), 활성 pane 이 바뀔 때 도는 `$effect`(line 789), 그리고 `registerTermFocus` /
`registerComposerFocus` 로 등록된 콜백(line 762·765). **조합 중에 포커스가 옮겨지면
WebKitGTK 가 조합 문자열을 다시 내보낼 수 있다**는 것이 가설 A 의 골자다.

---

## 5. 사용자에게 확인해야 하는 것 (미기입)

아래가 채워져야 6장의 순위가 확정된다. 각 질문은 특정 가설을 **죽이기 위해** 있다.

| # | 질문 | 무엇을 가르나 |
|---|------|---------------|
| (a) | 하단 입력창(composer)을 켜고 쓰시는가, 터미널에 직접 치는가? | 입력 경로를 절반으로 줄인다 |
| (b) | 되풀이가 터미널 화면에서 일어나는가, 입력창 안에서 일어나는가? | 4-1 의 비대칭이 원인인지 |
| (c) | 한글 조합 중에만 생기는가, 영문 타이핑에서도 생기는가? | **가설 A(IME)를 살리거나 죽인다** |
| (d) | 되돌아오는 게 방금 친 한 줄인가, 이전에 보낸 블록 전체인가? | 글자 단위 중복 vs 블록 재전송 |
| (e) | 그 순간 pane 상태 점의 색이 바뀌는가(특히 노랑=waiting)? | **가설 A-2(4-4 의 자동 포커스)를 직접 검증** |
| (f) | 터치패드 가운데 클릭 에뮬레이션을 쓰시는가? | 가설 B 를 살리거나 죽인다 |

**답: (미기입 — 2026-08-08 기준)**

---

## 6. 가설 (5장이 채워지기 전까지 순위는 잠정)

### 가설 A — IME 조합 중 포커스 이동으로 조합 문자열이 재전송된다

**근거**: 4-1(터미널 경로에 조합 가드 없음) + 4-4(사용자 조작 없이 포커스가 옮겨지는
코드가 있음). **"자동으로"라는 표현과 맞는 유일한 메커니즘**이다.

**확인 방법**: 질문 (c)(e). 영문만 칠 때는 안 생기고 한글에서만 생기면 A 가 유력.
증상이 나는 순간 상태 점이 노랑(waiting)으로 바뀐다면 4-4 의 effect 가 방아쇠다.

**손댈 자리(아직 손대지 말 것 — 확인 먼저)**: `attachCustomKeyEventHandler`(line 453)와
`onData`(line 719)에 `isComposing` 가드를 넣고, 4-4 의 effect 가 조합 중에는 포커스를
옮기지 않게 막는 것.

### 가설 B — 선택 자동 복사 + 가운데 클릭 붙여넣기의 되먹임

**근거**: 4-2(선택만 해도 클립보드가 덮어써짐) + 4-3(가운데 클릭이 붙여넣기).
사용자가 방금 친 글이 화면에 에코된 상태에서 어떤 이유로든 선택이 잡히면 그 글이
클립보드에 들어가고, 가운데 클릭 한 번에 그대로 되돌아온다.

**약점**: 물리적 클릭이 필요해 "자동으로"와 어긋난다. 질문 (f)가 이걸 가른다.

### 가설 C — 스크롤백 검색이 클립보드를 덮어쓴다

**근거**: 4-2 의 미확인 부분. xterm 의 SearchAddon 이 일치 구간을 선택하는 방식이면
Ctrl+Shift+F 로 검색어를 칠 때마다 클립보드가 매 글자 덮어써진다.

**확인 방법**: 검색창에 한 글자 치고 다른 곳에 Ctrl+V 해 보면 즉시 판명된다. (코드
수정 없이 확인 가능 — 다음 사람이 제일 먼저 해 볼 것.)

### 가설 D — 하단 입력창의 여러 줄 전송 경로가 두 번 돈다

**근거**: `sendDraft()` 는 여러 줄일 때 bracketed paste 로 감싸 보낸 뒤 **40ms 뒤에 Enter 를
따로** 보낸다(line 235~241). 이 40ms 사이에 다시 전송이 걸리거나 앱이 붙여넣기 블록을
아직 소화하지 못했다면 어긋날 수 있다.

**약점**: 사용자가 Enter 를 눌러야 시작된다. 질문 (a)(d)가 이걸 가른다.

---

## 7. 시도 기록 (다음 사람은 여기에 줄을 **추가**할 것)

| 날짜 | 커밋 | 무엇을 했나 | 결과 |
|------|------|-------------|------|
| 2026-06-12 | `830a1da` | Ctrl+V 분기에 `preventDefault` + 호스트에 `onpastecapture` 차단 | Ctrl+V 이중 붙여넣기는 해결. 현재 증상은 재발 |
| 2026-07-27 | `67fae64` | Shift+Enter 분기에 `preventDefault` — keypress 누출 차단 | Shift+Enter 제출 버그는 해결. 현재 증상과는 별건 |
| 시기 미상 | (주석만) | `handOff()` 의 포커스 이동을 `setTimeout(…, 0)` 으로 지연 | `//` 중복은 해결 |
| 2026-06-24 | `7c39317` | 입력 직후 1.5초 출력을 에코로 간주(상태 판정) | 입력 경로와 무관 — 중복 입력에는 효과 없음 |
| 2026-08-08 | (없음) | 코드 전수 조사 후 본 문서 작성. **코드 변경 없음** | 4-1 의 IME 가드 부재를 발견. 사용자 관찰 대기 |
| 2026-08-08 | (미커밋) | **가설 A 수정 시행** — IME 조합 중 자동 포커스 이동 차단 (8장) | ❌ **재발** — 가설 A 사망. 수정 자체는 별개 결함이라 유지 |
| 2026-08-08 | (미커밋) | **가설 E 수정 시행** — 타이핑 중 휠→방향키 변환 차단 (9장) | ⚠️ 입력창은 해결 ✅ / 터미널은 재발 ❌ — 원인은 `lastKeyAt` 을 찍는 순서 버그 |
| 2026-08-08 | (미커밋) | **가설 E 수정 2차** — 타이핑 시각을 캡처 단계 DOM 리스너로 이동 | ❌ 가설 E 사망(사용자가 터치패드를 쓰지 않음). 방어 조치로만 유지 |
| 2026-08-08 | (미커밋) | **진짜 원인 수정** — 조합 시작 시 xterm 숨은 textarea 비우기 (10장) | ✅ **해결 확인** — 사용자 확인 + 로그로 객관 검증 |

---

## 10. 진짜 원인과 해결 — xterm 의 IME 처리가 누적 버퍼를 통째로 재전송한다

### 원인

xterm 은 키 입력을 받으려고 화면 뒤에 숨은 `<textarea>` 를 둔다. 그 값은 **포커스가
빠질 때만** 비워지므로, 터미널에 포커스를 준 뒤로 친 글자가 계속 쌓인다. 문제는 IME 가
켜져 있을 때 도는 아래 코드다.

```ts
// node_modules/@xterm/xterm/src/browser/input/CompositionHelper.ts — _handleAnyTextareaChanges()
const oldValue = this._textarea.value;                 // 바뀌기 전 값
setTimeout(() => {
  const newValue = this._textarea.value;               // 바뀐 뒤 값
  const diff = newValue.replace(oldValue, '');         // 접두사 제거가 아니라 "문자열 찾아 바꾸기"
  if (newValue.length > oldValue.length)      triggerDataEvent(diff);      // 늘었으면 늘어난 만큼
  else if (newValue.length < oldValue.length) triggerDataEvent(DEL);       // 줄었으면 백스페이스
  else if (newValue !== oldValue)             triggerDataEvent(newValue);  // ← 길이 같고 내용만 다르면 "전체"
}, 0);
```

**한글은 한 글자를 제자리에서 바꿔 가며 완성한다.** `하` 에 받침 `ㄴ` 을 붙이면 `한` 이
되는데 **길이는 1 그대로이고 내용만 바뀐다.** 그래서 세 번째 분기에 정확히 걸리고, 그
순간 textarea 에 쌓여 있던 **"지금까지 친 것 전부"** 가 한 덩어리로 PTY 에 다시 들어간다.

두 번째 줄도 위험하다. `newValue.replace(oldValue, '')` 는 접두사를 떼는 것이 아니라
문자열을 찾아 바꾸는 것이라, 옛 값이 새 값 안에 없으면(제자리 수정이면 대개 없다)
아무것도 못 찾고 `diff` 가 새 값 **전체**가 된다.

이 하나로 관찰된 모든 사실이 설명된다. **영문에는 제자리 수정이 없어서** 안 터지고,
**하단 입력창은 xterm 을 아예 거치지 않아서**(평범한 textarea 에서 문장을 완성한 뒤 한
번에 PTY 로 보냄) 멀쩡했으며, **터치패드·붙여넣기·포커스 이동이 전혀 필요 없다.**

### 해결

xterm 자체는 고칠 수 없으므로(설치된 라이브러리), **조합이 시작되는 순간 그 textarea 를
비운다.**

```ts
// src/lib/Terminal.svelte — onMount 안
const resetImeBuffer = () => {
  if (xtermTextarea) xtermTextarea.value = "";       // 조합 시작 시 누적분을 지운다
};
host.addEventListener("compositionstart", resetImeBuffer, true);   // true = 캡처 단계
```

**캡처 단계로 단 것이 핵심이다.** 그래야 xterm 자신의 `compositionstart` 처리보다 먼저
돌고, xterm 은 비워진 값을 기준으로 조합 시작 위치를 0 으로 잡는다. 결과적으로 그
textarea 에는 항상 "지금 조합 중인 글자" 하나만 남으므로 누적 자체가 사라진다.

### 검증 (객관 데이터)

`input-probe.py` 로 35.8초간 한글을 타이핑한 로그를 기계적으로 분석했다.

```
기록된 덩어리 수    : 357
가장 큰 덩어리      : 3바이트  b'\xe3\x85\x81'
두 글자 이상 덩어리 : 0건        <-- 버그가 살아 있었다면 누적분이 한 덩어리로 찍혔을 것
방향키 등 이스케이프: 0건
```

**모든 덩어리가 한 글자짜리**이고 뭉쳐 들어온 것이 하나도 없다. 사용자도 증상이 사라졌다고
확인했다.

---

## 8. 2026-08-08 수정 — IME 조합 중 자동 포커스 이동 차단 (가설 A)

### 왜 이 수정인가

xterm 의 조합 처리 코드(`node_modules/@xterm/xterm/src/browser/input/CompositionHelper.ts`)를
직접 읽어서 메커니즘을 확정했다. 조합이 끝나면 xterm 은 자기 숨은 textarea 에서
**"조합 시작 위치부터 끝까지"** 를 잘라 PTY 로 보낸다.

```ts
// node_modules/@xterm/xterm/src/browser/input/CompositionHelper.ts — _finalizeComposition()
this._compositionPosition.start = this._textarea.value.length;   // compositionstart 때 기록
…
setTimeout(() => {
  input = this._textarea.value.substring(currentCompositionPosition.start);  // 여기부터 끝까지 전부
  if (input.length > 0) this._coreService.triggerDataEvent(input, true);
}, 0);
```

그리고 그 textarea 는 **blur 될 때만** 비워진다.

```ts
// node_modules/@xterm/xterm/src/browser/CoreBrowserTerminal.ts — _handleTextAreaBlur()
private _handleTextAreaBlur(): void {
  this.textarea!.value = '';    // 포커스가 빠지는 순간 통째로 비운다. 조합은 취소하지 않는다.
  …
}
```

즉 **조합이 진행 중일 때 포커스가 움직이면** 기록해 둔 시작 위치와 실제 textarea 값이
어긋나고, 그 어긋난 계산의 결과가 PTY 로 전송된다. 그래서 이미 친 글이 통째로 한 번 더
들어가거나 반대로 사라진다.

amux 는 이 함정을 특히 자주 밟는다. **사람이 아무것도 하지 않아도** 포커스를 옮기는
자리가 있기 때문이다 — 특히 엔진이 pane 상태를 `waiting` 으로 바꿀 때 도는 effect 는
사용자 조작이 전혀 없이 발동한다. 이것이 "자동으로"라는 사용자 표현과 맞아떨어지는
유일한 메커니즘이었다.

### 무엇을 바꿨나

`src/lib/Terminal.svelte` 에 조합 진행 여부를 담는 상태를 하나 두고, xterm 의 숨은
textarea 와 하단 입력창 양쪽에서 조합 시작·종료를 관찰한다.

```ts
// src/lib/Terminal.svelte
let imeComposing = $state(false);            // 조합이 진행 중인가
…
const xtermTextarea = term.textarea;         // term.open() 이 만든 숨은 textarea
xtermTextarea?.addEventListener("compositionstart", () => imeComposing = true);
xtermTextarea?.addEventListener("compositionend",   () => imeComposing = false);
```

그리고 **자동 포커스 이동 세 자리**를 이 값으로 막았다.

```ts
// ① 엔진이 waiting 으로 바꿀 때 — 사용자 조작이 없는 유일한 방아쇠 (before/after)
// before: if (w && !prevWaiting && untrack(() => composerFocused && draft === ""))
if (w && !prevWaiting && !imeComposing && untrack(() => composerFocused && draft === ""))  // ← 여기가 바뀐 곳

// ② 활성 pane 이 바뀔 때 도는 effect
if (imeComposing) return;                    // ← 여기가 바뀐 곳. 조합이 끝나면 effect 가 다시 돌아 마저 옮긴다
if (focused && term && !untrack(() => composerFocused)) term.focus();

// ③ focusTerm() 이 부르는 등록 콜백
if (!composerFocused && !imeComposing) term.focus();   // ← 여기가 바뀐 곳
```

키 해석기 두 곳에도 조합 가드를 넣었다. 조합 중에는 `keyCode` 가 229 로 뭉뚱그려 오고
`e.key` 도 실제 키와 다르게 실릴 수 있어, 단축키로 가로채면 조합이 끊긴다.

```ts
// src/lib/Terminal.svelte — attachCustomKeyEventHandler() 맨 앞
if (e.isComposing || e.keyCode === 229) return true;   // xterm 의 조합 처리에 그대로 맡긴다

// src/lib/keymap.ts — handleKey() 맨 앞
if (e.isComposing || e.keyCode === 229) return false;  // 단축키로 보지 않는다
```

`draft === ""` 만으로는 왜 부족했는지도 기록해 둔다 — **한글은 확정 전까지 글자가 IME
안에만 머물러서 `draft` 가 여전히 빈 문자열이다.** 그래서 "쓰던 글이 없으면 포커스를
넘긴다"는 기존 가드가 조합 중에는 그대로 통과해 버렸다.

### 검증 상태

`svelte-check` 0 errors. **실제 증상 재현 검증은 사용자만 가능하다** — 프로그램으로
보내는 입력(`amux send`)은 xterm 을 거치지 않고 PTY 로 직행하므로 IME 경로를 전혀
타지 않는다. 그래서 이 수정이 맞았는지는 사람이 한글을 쳐 봐야만 알 수 있다.

**이 수정 이후에도 증상이 재발하면**: 가설 A 는 죽은 것이고, 5장의 질문 (f)(터치패드
가운데 클릭)를 확인해 가설 B 로 넘어갈 것.

### 결과 — **가설 A 는 죽었다 (2026-08-08, 사용자 확인)**

사용자가 수정된 빌드로 시험했으나 **증상이 그대로 재발**했다. 따라서 IME 조합 중 포커스
이동은 이 증상의 원인이 아니다. 다만 위 수정 자체는 실재하는 결함(조합 중 포커스 이동 시
입력이 어긋남)을 막는 것이므로 **되돌리지 않고 남겨 둔다.** 다음 사람은 이 방향을 다시
파지 말 것.

배제 ④로 승격: **"IME 조합 중 자동 포커스 이동" 은 원인이 아니다.**

---

## 9. 2026-08-08 두 번째 수정 — 타이핑 중 휠이 방향키로 둔갑하는 문제 (가설 E)

### 이 가설이 어디서 나왔나

사용자의 "혹시 클로드 문제야?" 라는 물음이 실마리였다. Claude Code 는 **↑ 키를 받으면
직전에 보낸 프롬프트를 입력창에 통째로 되돌려 놓는다**(입력 이력 되부르기). 그렇다면
"이미 쓴 글이 통째로 다시 나타난다"는 증상은 **누군가 ↑ 를 보내고 있다**는 뜻이 된다.
그래서 "amux 가 사용자 모르게 ↑ 를 보내는 경로가 있는가" 를 찾았고, xterm 본체에서
발견했다.

```ts
// node_modules/@xterm/xterm/src/browser/CoreBrowserTerminal.ts — 휠 이벤트 처리
if (!this.buffer.hasScrollback) {
  // 스크롤백이 없는 화면(= vim·tmux 같은 전체화면 앱이 쓰는 "대체 화면 버퍼")에서는
  // 마우스 지원이 없는 앱도 휠로 스크롤할 수 있게, 휠을 방향키로 바꿔 보낸다.
  const sequence = C0.ESC + (applicationCursorKeys ? 'O' : '[') + (ev.deltaY < 0 ? 'A' : 'B');
  this.coreService.triggerDataEvent(sequence, true);   // ← PTY 로 ESC[A(위) / ESC[B(아래) 를 보낸다
  return this.cancel(ev, true);
}
```

즉 **전체화면 TUI 가 떠 있을 때 휠을 조금만 굴려도 앱은 방향키를 받는다.** 노트북
터치패드에서는 타이핑 도중 손바닥이나 손가락이 스치기만 해도 두 손가락 스크롤로 인식되고,
사용자는 아무 키도 누르지 않았다고 느낀다. 그런데 Claude Code 쪽에서는 ↑ 가 들어온 것이므로
직전 프롬프트를 입력창에 되돌려 놓는다 — **"자동으로 작성한 게 다시 붙여넣어지는" 그림이
정확히 완성된다.**

이 가설은 앞선 관찰과 전부 들어맞는다. 키를 누르지 않았는데(=자동으로) · 이전에 **작성한**
글이 · 한 덩어리로(=붙여넣기처럼) · **타이핑을 하다가**(손이 키보드에 있어 터치패드에 가까움)
나타난다. 그리고 IME 수정이 듣지 않은 이유도 설명된다 — 조합과는 아무 상관이 없다.

### 무엇을 바꿨나

`src/lib/Terminal.svelte` 에서 xterm 의 휠 처리를 가로챈다. `attachCustomWheelEventHandler`
는 xterm 의 공개 API 이고, `false` 를 돌려주면 그 휠 이벤트를 xterm 이 아예 처리하지 않는다.

```ts
// src/lib/Terminal.svelte — onMount 안
const TYPING_SCROLL_GUARD_MS = 800;   // 마지막 키 입력 후 이 시간 안의 휠은 "스친 것"으로 본다
let lastKeyAt = 0;
…
term.attachCustomWheelEventHandler((e) => {
  if (e.ctrlKey) return false;                                   // Ctrl+휠은 글꼴 확대/축소 — 호스트가 처리
  if (Date.now() - lastKeyAt < TYPING_SCROLL_GUARD_MS) return false;  // 타이핑 직후의 휠은 삼킨다
  return true;                                                   // 그 외에는 원래대로 xterm 에 맡긴다
});
```

`lastKeyAt` 은 두 곳에서 찍는다 — 터미널의 키 핸들러(`attachCustomKeyEventHandler` 맨 앞)와
하단 입력창의 `composerKey()`. 입력창에 치는 동안에도 손은 터미널 위를 지나가므로 둘 다
"타이핑 중"으로 봐야 한다.

**손을 멈추고 의도적으로 스크롤하는 경우는 800ms 창을 벗어나므로 정상 동작한다.** 즉
vim·less 안에서 휠로 스크롤하는 기능은 그대로 살아 있다.

### 이 수정이 맞았는지 가르는 결정적 실험

다음 검증에서는 추측을 줄이기 위해 **PTY 가 실제로 무엇을 받는지 눈으로 보는** 탭을 함께
띄운다. 터미널에서 `cat -v` 를 실행해 두면 받은 바이트가 그대로 화면에 찍히고, 제어문자는
`^[[A` 같은 보이는 형태로 나온다.

- 아무 키도 안 눌렀는데 `^[[A` 가 찍힌다 → **휠 → 방향키 변환이 범인**(가설 E 확정)
- 친 글자가 그 자리에서 두 번 찍힌다 → 원인은 Claude Code 아래쪽(amux/xterm)
- `cat -v` 에서는 멀쩡한데 Claude Code 에서만 재발한다 → 원인은 Claude Code 쪽

### 1차 검증 결과 (2026-08-08, 사용자) — **비대칭이 원인을 특정했다**

- **하단 입력창(composer)에 칠 때: 괜찮다** ✅
- **터미널에 직접 칠 때: 그대로 재발** ❌

이 비대칭이 **내 수정 자체의 버그**를 정확히 가리켰다. `lastKeyAt`(마지막 키 입력 시각)을
찍는 위치가 두 경로에서 달랐다.

```ts
// src/lib/Terminal.svelte — 입력창 쪽 composerKey() : 조기 반환보다 "앞"에서 찍는다 → 정상 동작
function composerKey(e: KeyboardEvent) {
  lastKeyAt = Date.now();                              // ← 먼저 찍고
  if (e.isComposing || e.keyCode === 229) return;      // ← 그 다음에 조합 가드
```

```ts
// src/lib/Terminal.svelte — 터미널 쪽 attachCustomKeyEventHandler() : 순서가 반대였다 (버그)
term.attachCustomKeyEventHandler((e) => {
  if (e.isComposing || e.keyCode === 229) return true; // ← 조합 중이면 여기서 나가버리고
  if (e.type === "keydown") lastKeyAt = Date.now();    // ← 이 줄에 영영 도달하지 못한다
```

**한글 조합 중의 키는 keyCode 가 229 로 온다.** 그래서 한글을 치는 동안에는 터미널 쪽
`lastKeyAt` 이 한 번도 갱신되지 않았고, 휠 가드가 통째로 죽어 있었다. 입력창 쪽은 순서가
맞아서 살아 있었다 — 사용자가 관찰한 "입력창은 괜찮고 터미널만 재발"이 정확히 이것이다.

### 2차 수정 — 타이핑 시각을 xterm 이 아니라 DOM 에서 직접 찍는다

xterm 내부 사정(조합 처리·조기 반환)에 의존하지 않도록, **캡처 단계의 DOM 리스너**로
옮겼다. 캡처 단계는 xterm 이 무엇을 하든 항상 먼저 실행된다.

```ts
// src/lib/Terminal.svelte — term.open(host) 직후
const stampKey = () => (lastKeyAt = Date.now());
host.addEventListener("keydown", stampKey, true);   // true = 캡처 단계
```

**아직 남은 불확실성**: 위 비대칭은 "휠→방향키" 가설과 "xterm 입력 경로 자체가 문제"
가설 **양쪽 모두와 양립한다**(입력창은 xterm 을 아예 거치지 않으므로). 이를 가르는 것은
`cat -v` 탭뿐이다 — 1차 검증에서 이 탭 결과는 보고되지 않았다. **다음 검증에서는 반드시
`cat -v` 탭 결과를 받을 것.**

### 가설 E 도 죽었다 (2026-08-08, 사용자)

사용자가 **키보드로만 치고 있어 터치패드에 닿을 일이 없다**고 확인했다. 휠 이벤트가
발생할 수 없으므로 가설 E 는 원인이 아니다. 다만 "손을 멈추지 않은 상태의 휠이 ↑ 로
둔갑해 직전 프롬프트가 되살아나는" 사고 자체는 실재하므로, 폭이 좁은 방어(마지막 키
입력 후 800ms)로 **남겨 두었다**. 원인 수정이 아니라 별개의 예방 조치다.

배제 ⑤로 승격: **"휠 → 방향키 변환" 은 이 증상의 원인이 아니다.**

### 검사 도구 설계 실패 — `cat -v` 는 이 검증에 쓸 수 없다

1차 검증에서 `cat -v` 를 원시 입력 확인용으로 띄웠는데, **`cat` 은 한 줄을 읽으면 그
줄을 그대로 되돌려 출력한다.** 여기에 터미널 자체의 에코가 겹쳐, 정상 동작만으로도 같은
줄이 두 번 보인다. 그래서 "진짜 중복"과 "원래 그런 것"을 구분할 수 없다 — 실제로 사용자는
이 탭에서도 증상이 보인다고 보고했으나, 그것이 `cat` 의 정상 동작이었을 가능성을 배제할 수
없었다. **다음에 같은 검증이 필요하면 `cat` 을 쓰지 말 것.**

대신 만든 것이 `input-probe.py`(스크래치패드) 다. 터미널을 raw 모드로 바꿔 **에코를 끄고**,
도착한 바이트 덩어리마다 경과시간과 파이썬 `repr` 을 찍으며 같은 내용을 로그 파일에
남긴다. 사람이 눈으로 판독할 필요 없이 기계적으로 분석할 수 있다는 것이 핵심이다.
