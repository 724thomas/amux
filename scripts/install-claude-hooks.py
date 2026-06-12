#!/usr/bin/env python3
"""~/.claude/settings.json에 cmux 상태 연동 hook을 설치합니다.

기존 설정은 보존하고 hooks 키만 병합하며, 원본은 settings.json.bak-cmux로
백업합니다. cmux pane 밖의 Claude 세션에서는 hook이 조용히 무시됩니다
(`|| true`).
"""

import json
import shutil
from pathlib import Path

SETTINGS = Path.home() / ".claude" / "settings.json"
BACKUP = SETTINGS.with_suffix(".json.bak-cmux")


def hook(cmd: str):
    return [{"matcher": "", "hooks": [{"type": "command", "command": cmd}]}]


def main():
    settings = json.loads(SETTINGS.read_text()) if SETTINGS.exists() else {}
    shutil.copy(SETTINGS, BACKUP)

    hooks = settings.setdefault("hooks", {})
    hooks["SessionStart"] = hook("cmux notify --kind idle 2>/dev/null || true")
    hooks["UserPromptSubmit"] = hook("cmux notify --kind progress 2>/dev/null || true")
    hooks["PostToolUse"] = hook("cmux notify --kind progress 2>/dev/null || true")
    hooks["Notification"] = hook(
        "cmux notify --kind attention --from-claude-hook 2>/dev/null || true"
    )
    hooks["Stop"] = hook(
        "cmux notify --kind done --title 'Claude Code' --body '작업이 끝났습니다' 2>/dev/null || true"
    )

    SETTINGS.write_text(json.dumps(settings, indent=2, ensure_ascii=False) + "\n")
    print(f"설치 완료. 백업: {BACKUP}")
    print("실행 중인 Claude Code 세션은 재시작해야 hook이 적용됩니다.")


if __name__ == "__main__":
    main()
