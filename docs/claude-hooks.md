# Claude Code 알림·상태 연동

amux pane 안에서 실행되는 모든 프로세스는 `AMUX_PANE_ID` / `AMUX_SOCKET` 환경
변수를 물려받으므로, Claude Code hook에서 `amux notify`를 그대로 호출하면
자기 pane을 알아서 찾아갑니다.

## 설치 (권장)

```bash
python3 scripts/install-claude-hooks.py
```

또는 `~/.claude/settings.json`에 직접 추가:

```json
{
  "hooks": {
    "SessionStart": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "amux notify --kind idle 2>/dev/null || true" } ] }
    ],
    "UserPromptSubmit": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "amux notify --kind progress 2>/dev/null || true" } ] }
    ],
    "PostToolUse": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "amux notify --kind progress 2>/dev/null || true" } ] }
    ],
    "Notification": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "amux notify --kind attention --from-claude-hook 2>/dev/null || true" } ] }
    ],
    "Stop": [
      { "matcher": "", "hooks": [
        { "type": "command", "command": "amux notify --kind done --title 'Claude Code' --body '작업이 끝났습니다' 2>/dev/null || true" } ] }
    ]
  }
}
```

## 상태 매핑

Claude Code 같은 TUI는 대기 중에도 화면을 계속 다시 그리므로 출력 침묵
휴리스틱이 통하지 않습니다. hook이 한 번이라도 발화한 pane은 hook이 상태의
단일 소스가 됩니다:

| hook | amux 상태 | 색 |
|---|---|---|
| SessionStart (claude 시작) | idle | 파랑 |
| UserPromptSubmit / PostToolUse | processing… | 빨강 |
| Stop | processed → (열람 후) idle | 초록 → 파랑 |
| Notification (권한/질문 대기) | waiting | 노랑 |

- **Notification hook**: `--from-claude-hook`이 hook의 JSON 페이로드(stdin)
  에서 메시지를 직접 추출하므로 `jq`가 필요 없습니다.
- hook이 없는 pane은 출력 휴리스틱으로 동작합니다: 입력 직후 1.5초 내의
  출력(에코·프롬프트 다시 그리기)은 무시하고, 실제 작업 출력이 4초 멎거나
  포그라운드 프로세스가 끝나면 완료로 판정합니다. 상태는 항상
  processing / processed / idle / waiting 4가지 중 하나입니다.

## 동작

amux 앱은 알림을 받으면:

- 해당 pane을 **지금 보고 있으면** (활성 워크스페이스 + 활성 pane + 창 포커스)
  아무것도 하지 않습니다 — 이미 보고 있으니까.
- 그 외에는: GNOME 데스크톱 알림 + pane 하이라이트 링(3초) + 사이드바
  뱃지·메시지. pane을 클릭하거나 창에 돌아오면 뱃지가 사라집니다.

## 다른 채널

hook 없이도 표준 이스케이프 시퀀스로 알림을 만들 수 있습니다:

```bash
printf '\a'                                  # BEL
printf '\033]9;빌드 완료\007'                 # OSC 9 (iTerm2 스타일)
printf '\033]777;notify;제목;본문\033\\'      # OSC 777 (urxvt 스타일)
```

Codex CLI는 `~/.codex/config.toml`의 `notify` 키로 같은 연동이 가능합니다:

```toml
notify = ["amux", "notify", "--kind", "attention", "--title", "Codex"]
```
