# Claude Code 알림 연동

cmux pane 안에서 실행되는 모든 프로세스는 `CMUX_PANE_ID` / `CMUX_SOCKET` 환경
변수를 물려받으므로, Claude Code hook에서 `cmux notify`를 그대로 호출하면
자기 pane을 알아서 찾아갑니다.

## 설정

`~/.claude/settings.json`에 추가:

```json
{
  "hooks": {
    "Notification": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cmux notify --kind attention --from-claude-hook"
          }
        ]
      }
    ],
    "Stop": [
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "cmux notify --kind done --title 'Claude Code' --body '작업이 끝났습니다'"
          }
        ]
      }
    ]
  }
}
```

- **Notification hook**: Claude가 권한 승인 등 입력을 기다릴 때 발화.
  `--from-claude-hook`이 hook의 JSON 페이로드(stdin)에서 메시지를 직접
  추출하므로 `jq`가 필요 없습니다.
- **Stop hook**: Claude의 응답이 끝났을 때 발화.

## 동작

cmux 앱은 알림을 받으면:

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
notify = ["cmux", "notify", "--kind", "attention", "--title", "Codex"]
```
