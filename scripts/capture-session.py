#!/usr/bin/env python3
"""지금 켜져 있는 amux의 배치를 세션 파일로 찍어 냅니다 (읽기 전용).

세션 자동 저장이 들어간 뒤로는 앱이 1초마다 스스로 `~/.config/amux/session.json` 에
배치를 기록하므로 이 스크립트가 없어도 됩니다. 필요한 경우는 두 가지입니다.

1. **자동 저장이 없던 버전에서 올라올 때.** 새 빌드를 깔려면 지금 켜져 있는 amux를
   껐다 켜야 하는데, 그 순간에는 세션 파일이 아직 없으므로 지금 열려 있는 배치가
   그대로 사라집니다. 끄기 **직전에** 이 스크립트를 돌리면 그 배치가 파일로 남아,
   새 빌드를 처음 켤 때 복구 카드가 뜹니다.
2. 지금 배치를 따로 한 벌 떠 두고 싶을 때 (`-o` 로 다른 경로에 저장).

동작: amux 소켓에 `workspace.list` 와 `pane.list` 두 개의 **읽기 전용** JSON-RPC를
보내 현재 상태를 받아 오고, 그것을 엔진이 쓰는 것과 같은 모양의 JSON으로 옮겨 적습니다.
켜져 있는 amux에는 아무 변화도 주지 않습니다.

    python3 scripts/capture-session.py            # ~/.config/amux/session.json 에 저장
    python3 scripts/capture-session.py -o /tmp/a.json
"""

import argparse
import json
import os
import re
import socket
import sys
import time
from pathlib import Path

FORMAT_VERSION = 1
# 엔진이 자동으로 붙이는 이름들. 복구 뒤 Ctrl+T 가 같은 이름을 또 내주지 않도록,
# 이미 쓰인 번호 중 가장 큰 값을 세어 카운터로 넘긴다.
AUTO_TAB = re.compile(r"^탭 (\d+)$")
AUTO_WS = re.compile(r"^워크스페이스 (\d+)$")


WINDOWS = os.name == "nt"


def socket_path() -> Path:
    """amux_protocol::default_socket_name() 과 같은 자리를 가리켜야 한다."""
    explicit = os.environ.get("AMUX_SOCKET")
    if WINDOWS:
        # 윈도우에는 파일시스템 경로가 없다. 이름 하나가 named pipe 로 매핑되고,
        # 이름은 계정별로 나뉜다 (named pipe 는 기기 전체에서 공유되므로).
        name = explicit or "amux-%s.sock" % "".join(
            c if c.isascii() and c.isalnum() else "_" for c in os.environ.get("USERNAME", "user")
        )
        return Path(name if name.startswith(r"\\") else r"\\.\pipe" + "\\" + name)
    if explicit:
        return Path(explicit)
    if runtime := os.environ.get("XDG_RUNTIME_DIR"):
        return Path(runtime) / "amux" / "amux.sock"
    return Path(f"/tmp/amux-{os.geteuid()}") / "amux.sock"


def session_path() -> Path:
    if explicit := os.environ.get("AMUX_SESSION_FILE"):
        return Path(explicit)
    # 엔진의 session_path() 와 같은 순서: XDG_CONFIG_HOME → HOME → %APPDATA%.
    base = os.environ.get("XDG_CONFIG_HOME") or os.environ.get("HOME")
    if base is None:
        base = os.environ.get("APPDATA") if WINDOWS else None
    else:
        base = Path(base) / ".config"
    return Path(base or Path.home() / ".config") / "amux" / "session.json"


def exchange(sock_path: Path, request: bytes) -> bytes:
    """줄 하나를 보내고 줄 하나를 받는다. 유닉스 소켓이냐 named pipe 냐만 다르다."""
    if WINDOWS:
        # named pipe 는 파일처럼 열어 읽고 쓸 수 있다. 버퍼링을 끄는 것이
        # 중요하다 — 요청이 버퍼에 남아 있으면 서버는 영영 응답하지 않는다.
        with open(sock_path, "r+b", buffering=0) as pipe:
            pipe.write(request)
            return read_line(pipe.read)
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(str(sock_path))
        s.sendall(request)
        return read_line(s.recv)


def read_line(recv) -> bytes:
    buf = b""
    while not buf.endswith(b"\n"):
        chunk = recv(65536)
        if not chunk:
            break
        buf += chunk
    return buf


def rpc(sock_path: Path, method: str):
    """읽기 전용 JSON-RPC 한 번. 연결마다 새로 열고 닫는다 (CLI와 같은 방식)."""
    request = (json.dumps({"jsonrpc": "2.0", "id": 1, "method": method}) + "\n").encode()
    reply = json.loads(exchange(sock_path, request).decode())
    if "error" in reply and reply["error"]:
        raise SystemExit(f"amux가 {method} 를 거부했습니다: {reply['error']}")
    return reply["result"]


def to_saved_layout(node, panes_by_id):
    """엔진의 LayoutNode(잎이 pane id)를 저장용 트리로 바꾼다.

    잎에는 그 pane 의 디렉터리와, hook 이 알려 준 Claude 대화 id 를 적는다.
    대화 id 는 hook 이 보고해 준 pane 에만 있고, 없으면 null 이다.
    """
    if node["type"] == "leaf":
        pane = panes_by_id.get(node["pane"], {})
        return {
            "type": "leaf",
            "cwd": (pane.get("meta") or {}).get("cwd"),
            "claude_session": pane.get("claude_session"),
        }
    return {
        "type": "split",
        "axis": node["axis"],
        "ratio": node["ratio"],
        "first": to_saved_layout(node["first"], panes_by_id),
        "second": to_saved_layout(node["second"], panes_by_id),
    }


def panes_in_order(node):
    """layout::panes() 와 같은 순서. 저장되는 active_pane 은 이 목록의 위치다."""
    if node["type"] == "leaf":
        return [node["pane"]]
    return panes_in_order(node["first"]) + panes_in_order(node["second"])


def highest_auto(names, pattern) -> int:
    """`탭 3` 처럼 자동으로 붙은 이름 중 가장 큰 번호. 하나도 없으면 0."""
    return max((int(m.group(1)) for n in names if (m := pattern.match(n))), default=0)


def tabs_of(ws):
    """워크스페이스의 탭 목록.

    탭 계층은 v0.5.0 에서 들어왔다. 이 스크립트의 존재 이유가 "탭도 자동 저장도
    없던 버전에서 올라올 때 배치를 건져 내는 것"이므로, 그보다 옛 앱이 돌려주는
    모양 — 워크스페이스가 곧 분할 트리 하나인 모양 — 도 받아야 한다. 그때는
    그 트리를 탭 하나로 감싼다.
    """
    if "tabs" in ws:
        return ws["tabs"]
    return [
        {
            "id": ws["id"],
            "name": "탭 1",
            "layout": ws["layout"],
            "active_pane": ws.get("active_pane"),
        }
    ]


def build_session(workspaces, panes) -> dict:
    panes_by_id = {p["id"]: p for p in panes}
    saved_workspaces = []
    for ws in workspaces:
        tabs = []
        for tab in tabs_of(ws):
            order = panes_in_order(tab["layout"])
            active = tab.get("active_pane")
            tabs.append(
                {
                    "name": tab["name"],
                    "layout": to_saved_layout(tab["layout"], panes_by_id),
                    "active_pane": order.index(active) if active in order else 0,
                }
            )
        tab_ids = [t["id"] for t in tabs_of(ws)]
        saved_workspaces.append(
            {
                "name": ws["name"],
                "tabs": tabs,
                "active_tab": tab_ids.index(ws["active_tab"]) if ws.get("active_tab") in tab_ids else 0,
                "tab_created_count": highest_auto([t["name"] for t in tabs], AUTO_TAB),
            }
        )
    return {
        "version": FORMAT_VERSION,
        "saved_at_ms": int(time.time() * 1000),
        "workspaces": saved_workspaces,
        # workspace.list 는 "어느 워크스페이스가 화면에 떠 있었는지"를 알려주지 않는다.
        # 복구하면 첫 번째 워크스페이스에서 시작한다.
        "active_workspace": 0 if saved_workspaces else None,
        "workspace_created_count": highest_auto([w["name"] for w in workspaces], AUTO_WS),
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("-o", "--output", type=Path, default=None, help="저장 경로 (기본: ~/.config/amux/session.json)")
    ap.add_argument("-f", "--force", action="store_true", help="이미 있는 파일도 덮어씁니다")
    args = ap.parse_args()

    sock = socket_path()
    # named pipe 는 exists() 로 확인되지 않는다 (파일시스템에 없으므로). 어차피
    # 열어 보면 바로 알 수 있으니, 양쪽 모두 연결 실패를 같은 문구로 옮긴다.
    if not WINDOWS and not sock.exists():
        print(f"amux 소켓이 없습니다: {sock}\namux가 켜져 있는지 확인하세요.", file=sys.stderr)
        return 1

    try:
        workspaces = rpc(sock, "workspace.list")
        panes = rpc(sock, "pane.list")
    except OSError as e:
        print(f"amux 에 연결하지 못했습니다 ({sock}): {e}\namux가 켜져 있는지 확인하세요.", file=sys.stderr)
        return 1
    session = build_session(workspaces, panes)
    if not session["workspaces"]:
        print("열려 있는 워크스페이스가 없어 저장할 배치가 없습니다.", file=sys.stderr)
        return 1

    out = args.output or session_path()
    if out.exists() and not args.force:
        print(f"{out} 이 이미 있습니다. 덮어쓰려면 --force 를 붙이세요.", file=sys.stderr)
        return 1

    out.parent.mkdir(parents=True, exist_ok=True)
    # 엔진과 같은 방식으로 원자적으로 쓴다: 임시 파일에 다 쓰고 이름만 바꿔 끼운다.
    tmp = out.with_suffix(".json.tmp")
    tmp.write_text(json.dumps(session, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp.replace(out)

    tab_count = sum(len(w["tabs"]) for w in session["workspaces"])
    pane_count = sum(len(panes_in_order(t["layout"])) for w in workspaces for t in tabs_of(w))
    print(f"저장했습니다: {out}")
    print(f"  워크스페이스 {len(session['workspaces'])}개, 탭 {tab_count}개, 터미널 {pane_count}개")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
