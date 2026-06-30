#!/usr/bin/env python3
"""~/.claude/settings.json에 amux 상태 연동 hook을 설치합니다 (리눅스·맥·윈도우 공용).

기존 설정은 보존하고 hooks 키만 병합하며, 원본이 있으면 settings.json.bak-amux로
백업합니다. amux pane 밖의 Claude 세션에서는 `amux notify`가 앱에 연결하지 못하면
조용히 종료(exit 0)하므로, hook이 세션을 깨지 않습니다 — 그래서 셸별 `2>/dev/null
|| true` 같은 군더더기 없이 모든 OS에서 같은 명령을 씁니다.
"""

import json
import os
import shutil
from pathlib import Path

SETTINGS = Path.home() / ".claude" / "settings.json"
BACKUP = SETTINGS.with_suffix(".json.bak-amux")

# cmd.exe는 큰따옴표로, POSIX 셸은 작은따옴표로 인자를 감싼다.
Q = '"' if os.name == "nt" else "'"


def hook(cmd: str):
    return [{"matcher": "", "hooks": [{"type": "command", "command": cmd}]}]


def main():
    existed = SETTINGS.exists()
    settings = json.loads(SETTINGS.read_text()) if existed else {}
    if existed:
        shutil.copy(SETTINGS, BACKUP)

    hooks = settings.setdefault("hooks", {})
    hooks["SessionStart"] = hook("amux notify --kind idle")
    hooks["UserPromptSubmit"] = hook("amux notify --kind progress")
    hooks["PostToolUse"] = hook("amux notify --kind progress")
    hooks["Notification"] = hook("amux notify --kind attention --from-claude-hook")
    hooks["Stop"] = hook(
        f"amux notify --kind done --title {Q}Claude Code{Q} --body {Q}작업이 끝났습니다{Q}"
    )

    SETTINGS.parent.mkdir(parents=True, exist_ok=True)
    SETTINGS.write_text(json.dumps(settings, indent=2, ensure_ascii=False) + "\n")
    if existed:
        print(f"설치 완료. 백업: {BACKUP}")
    else:
        print(f"설치 완료. 새 설정 파일 생성: {SETTINGS}")
    print("실행 중인 Claude Code 세션은 재시작해야 hook이 적용됩니다.")


if __name__ == "__main__":
    main()
