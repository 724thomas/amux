// Small text helpers for the chat view: one-line summaries of tool calls and
// bounded previews of long inputs and outputs.

type Input = Record<string, unknown>;

const str = (v: unknown): string => (typeof v === "string" ? v : v == null ? "" : String(v));

/** `/home/me/x/y` → `~/x/y`. */
/** A path for display: relative to `base` (the chat's folder) when inside it,
 *  else with the home folder written as `~`. */
export function shortPath(path: string, base: string | null = null): string {
  if (base) {
    const prefix = base.endsWith("/") ? base : base + "/";
    if (path.startsWith(prefix) && path.length > prefix.length) return path.slice(prefix.length);
  }
  return path.replace(/^\/home\/[^/]+/, "~").replace(/^\/Users\/[^/]+/, "~");
}

function firstLine(text: string, max = 120): string {
  const line = text.split("\n").find((l) => l.trim()) ?? "";
  const more = text.trim().includes("\n") ? " …" : "";
  return line.length > max ? line.slice(0, max) + "…" : line + more;
}

/** One line saying what a tool call does, for its row in the transcript. */
export function toolSummary(name: string, input: Input, base: string | null = null): string {
  switch (name) {
    case "Bash":
      return firstLine(str(input.command));
    case "Read": {
      const range = input.offset ? ` (${input.offset}행부터)` : "";
      return shortPath(str(input.file_path), base) + range;
    }
    case "Write":
    case "Edit":
    case "MultiEdit":
      return shortPath(str(input.file_path), base);
    case "NotebookEdit":
      return shortPath(str(input.notebook_path), base);
    case "Glob":
      return str(input.pattern) + (input.path ? `  (${shortPath(str(input.path), base)})` : "");
    case "Grep":
      return (
        str(input.pattern) +
        (input.glob ? `  [${str(input.glob)}]` : "") +
        (input.path ? `  (${shortPath(str(input.path), base)})` : "")
      );
    case "WebFetch":
      return str(input.url);
    case "WebSearch":
      return str(input.query);
    case "Task":
    case "Agent":
      return firstLine(str(input.description || input.prompt)) + (input.subagent_type ? `  [${str(input.subagent_type)}]` : "");
    case "Skill":
      return str(input.skill || input.command);
    case "ToolSearch":
      return str(input.query);
    case "AskUserQuestion":
      return Array.isArray(input.questions) ? `질문 ${input.questions.length}개` : "";
    default: {
      for (const v of Object.values(input)) if (typeof v === "string" && v.trim()) return firstLine(v);
      const json = JSON.stringify(input);
      return json === "{}" ? "" : json.length > 120 ? json.slice(0, 120) + "…" : json;
    }
  }
}

/** Cut a long text to `max` characters, saying how much was left out. */
export function clip(text: string, max: number): string {
  if (text.length <= max) return text;
  return text.slice(0, max) + `\n… (이하 ${(text.length - max).toLocaleString()}자 생략)`;
}

/** Pretty JSON of a tool input, minus the fields shown elsewhere. */
export function inputJson(input: Input): string {
  try {
    return JSON.stringify(input, null, 2);
  } catch {
    return String(input);
  }
}

export function seconds(ms: number | null): string {
  if (ms == null) return "";
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}초`;
  const m = Math.floor(s / 60);
  return `${m}분 ${Math.round(s - m * 60)}초`;
}
