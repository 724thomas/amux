// Markdown for Claude's replies. The output goes into `{@html}` inside the
// app's own webview, where page script can reach the Tauri IPC, so nothing a
// reply contains may become live markup:
// - raw HTML in the source is shown as text, never parsed;
// - links keep only http(s)/mailto targets (`javascript:` would run code), and
//   ChatView opens them in the system browser instead of navigating amux away;
// - images become links: loading arbitrary remote images is a tracking pixel.
// Mermaid and SVG code blocks get a placeholder that ChatView turns into a
// picture once the reply is complete.
import { Marked, type Tokens } from "marked";

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

const SAFE_LINK = /^(https?:|mailto:)/i;

const md = new Marked({ gfm: true, breaks: false, async: false });

md.use({
  renderer: {
    html(token: Tokens.HTML | Tokens.Tag) {
      return escapeHtml(token.text);
    },
    link(token: Tokens.Link) {
      const text = this.parser.parseInline(token.tokens);
      const href = (token.href ?? "").trim();
      if (!SAFE_LINK.test(href)) return text;
      const title = token.title ? ` title="${escapeHtml(token.title)}"` : "";
      return `<a href="${escapeHtml(href)}"${title}>${text}</a>`;
    },
    image(token: Tokens.Image) {
      const href = (token.href ?? "").trim();
      const alt = escapeHtml(token.text || "이미지");
      if (!/^https?:/i.test(href)) return `[${alt}]`;
      return `<a href="${escapeHtml(href)}">[이미지: ${alt}]</a>`;
    },
    code(token: Tokens.Code) {
      const lang = (token.lang ?? "").trim().split(/\s+/)[0].toLowerCase();
      // Pictures get a placeholder that ChatView swaps for the drawing once
      // the reply is complete; until then (and if drawing fails) the source
      // shows as code.
      const kind = lang === "mermaid" ? "mermaid-block" : lang === "svg" ? "svg-block" : null;
      if (!kind) return false;
      return (
        `<div class="${kind}" data-src="${escapeHtml(token.text)}">` +
        `<pre><code>${escapeHtml(token.text)}</code></pre></div>`
      );
    },
  },
});

export function renderMarkdown(source: string): string {
  try {
    return md.parse(source) as string;
  } catch {
    return `<pre>${escapeHtml(source)}</pre>`;
  }
}
