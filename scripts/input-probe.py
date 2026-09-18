#!/usr/bin/env python3
"""pane 에 실제로 도착한 입력 바이트를 그대로 찍어 보는 검사 도구.

`docs/known-issues/duplicate-input.md` 가 요구하는 도구입니다. "한글을 치면 글자가
사라진다/두 번 들어간다" 류의 증상은 눈으로는 원인을 가릴 수 없습니다. 화면에 보이는
것은 터미널이 그려 준 결과이지, amux 가 PTY 로 흘려보낸 것이 아니기 때문입니다.

**`cat -v` 를 쓰지 마세요.** `cat` 은 한 줄을 읽으면 그 줄을 되돌려 출력하고, 거기에
터미널 자체의 에코가 겹쳐 정상 동작만으로도 같은 줄이 두 번 보입니다. 그래서 "진짜
중복"과 "원래 그런 것"을 구분할 수 없습니다 (그 문서 9장의 실패 기록).

이 스크립트는 에코를 끄고, 도착한 덩어리마다 경과시간과 repr 을 한 줄씩 찍습니다.
찍히는 것은 오직 도착한 것뿐이라 화면이 곧 증거입니다.

    python scripts/input-probe.py            # 로그: %TEMP%(또는 /tmp)/amux-input-probe.log
    python scripts/input-probe.py -o out.log

끝내려면 Ctrl+] 를 누르세요 (Ctrl+C 는 그 자체가 입력이라 기록 대상입니다).

읽는 법 — 한글을 평소 속도로 "한글날" 처럼 쳐 보고:
  · 완성된 글자가 하나씩 차례로 찍힌다        → 입력 경로 정상
  · 친 글자가 아예 안 찍힌다                  → 조합 중간에 삼켜졌다 (전송 누락)
  · '\\x7f' 가 끼어 있다                       → 백스페이스가 섞여 들어갔다 (지움)
  · 이미 친 문장이 한 덩어리로 다시 찍힌다     → 누적 버퍼 재전송
"""

import argparse
import os
import sys
import time
from pathlib import Path

WINDOWS = os.name == "nt"
QUIT = "\x1d"  # Ctrl+]


def default_log() -> Path:
    base = Path(os.environ.get("TEMP") or "/tmp") if WINDOWS else Path("/tmp")
    return base / "amux-input-probe.log"


class RawInput:
    """에코 없이 한 글자씩 읽는다. 들어오는 즉시 — 줄 단위로 모으지 않는다."""

    def __enter__(self):
        if WINDOWS:
            # msvcrt 는 콘솔에서 직접 읽으므로 줄 편집도 에코도 거치지 않는다.
            import msvcrt

            self._getch = msvcrt.getwch
            self._restore = None
            return self
        import termios
        import tty

        self._fd = sys.stdin.fileno()
        self._saved = termios.tcgetattr(self._fd)
        tty.setraw(self._fd)
        self._restore = lambda: termios.tcsetattr(self._fd, termios.TCSADRAIN, self._saved)
        self._getch = lambda: sys.stdin.read(1)
        return self

    def __exit__(self, *_):
        if self._restore:
            self._restore()

    def read(self) -> str:
        return self._getch()


def describe(ch: str) -> str:
    """무엇이 도착했는지 한 줄로. 조용한 제어문자는 이름을 붙여 준다."""
    names = {"\x7f": "DEL(백스페이스)", "\x08": "BS", "\r": "CR(엔터)", "\n": "LF", "\x1b": "ESC"}
    note = names.get(ch, "")
    return f"{ch!r:<12} U+{ord(ch):04X} {note}"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-o", "--output", type=Path, default=None, help="로그 경로")
    args = ap.parse_args()
    out = args.output or default_log()

    print("입력 검사 시작. 평소 속도로 한글을 쳐 보세요 (예: 한글날 아침).")
    print(f"끝내려면 Ctrl+]    로그: {out}")
    print("-" * 60, flush=True)

    lines: list[str] = []
    start = time.monotonic()
    last = start
    try:
        with RawInput() as raw:
            while True:
                ch = raw.read()
                if ch == QUIT:
                    break
                now = time.monotonic()
                # 직전 입력과의 간격. 사람이 친 것인지, 한 덩어리로 쏟아진 것인지를
                # 가르는 값이라 같이 찍는다 (같은 ms 에 여럿 = 한 번에 들어온 것).
                line = f"{now - start:7.3f}s  +{(now - last) * 1000:6.1f}ms  {describe(ch)}"
                last = now
                lines.append(line)
                # raw 모드에서는 개행도 직접 넣어야 한다.
                sys.stdout.write(line + "\r\n")
                sys.stdout.flush()
    except KeyboardInterrupt:
        pass

    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print("-" * 60)
    print(f"{len(lines)}개 기록, 저장: {out}")
    dels = sum(1 for l in lines if "DEL(" in l)
    if dels:
        print(f"  ⚠ 백스페이스(DEL) {dels}개가 입력에 섞여 있습니다 — 지움 계열 버그.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
