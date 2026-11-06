// Reading a tool call's input while Claude is still writing it.
//
// With --include-partial-messages the input of a tool call arrives as JSON
// text in pieces (`input_json_delta`), e.g. `{"file_path": "/src/a.ts", "ol`.
// To show an edit as it is being typed, this reads the top-level string fields
// out of such an incomplete object: finished fields whole, the last one as far
// as it has arrived. Non-string values are skipped.

export interface PartialField {
  value: string;
  /** The closing quote has arrived. */
  done: boolean;
}

export function partialFields(text: string): Record<string, PartialField> {
  const out: Record<string, PartialField> = {};
  const n = text.length;
  let i = text.indexOf("{");
  if (i < 0) return out;
  i++;

  const skipSpace = () => {
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c !== 32 && c !== 9 && c !== 10 && c !== 13) break;
      i++;
    }
  };

  // Reads a JSON string starting at the opening quote at `i`.
  const readString = (): PartialField => {
    i++;
    let value = "";
    let start = i;
    while (i < n) {
      const c = text.charCodeAt(i);
      if (c === 34 /* " */) {
        value += text.slice(start, i);
        i++;
        return { value, done: true };
      }
      if (c === 92 /* \ */) {
        value += text.slice(start, i);
        if (i + 1 >= n) {
          i = n;
          return { value, done: false };
        }
        const e = text[i + 1];
        if (e === "u") {
          if (i + 6 > n) {
            i = n;
            return { value, done: false };
          }
          value += String.fromCharCode(parseInt(text.slice(i + 2, i + 6), 16));
          i += 6;
        } else {
          value += e === "n" ? "\n" : e === "t" ? "\t" : e === "r" ? "\r" : e === "b" ? "\b" : e === "f" ? "\f" : e;
          i += 2;
        }
        start = i;
        continue;
      }
      i++;
    }
    value += text.slice(start, n);
    return { value, done: false };
  };

  // Skips a non-string value; false if it runs past the end of the text.
  const skipValue = (): boolean => {
    let depth = 0;
    while (i < n) {
      const ch = text[i];
      if (ch === '"') {
        if (!readString().done) return false;
        continue;
      }
      if (ch === "{" || ch === "[") depth++;
      else if (ch === "}" || ch === "]") {
        if (depth === 0) return true;
        depth--;
        if (depth === 0) {
          i++;
          return true;
        }
      } else if (ch === "," && depth === 0) return true;
      i++;
    }
    return false;
  };

  while (i < n) {
    skipSpace();
    if (i >= n || text[i] === "}") break;
    if (text[i] === ",") {
      i++;
      continue;
    }
    if (text[i] !== '"') break;
    const key = readString();
    if (!key.done) break;
    skipSpace();
    if (text[i] !== ":") break;
    i++;
    skipSpace();
    if (i >= n) break;
    if (text[i] === '"') {
      const field = readString();
      out[key.value] = field;
      if (!field.done) break;
    } else if (!skipValue()) break;
  }
  return out;
}

/** The value of a string field once it has fully arrived, else null. */
export function completeField(text: string, key: string): string | null {
  const field = partialFields(text)[key];
  return field?.done ? field.value : null;
}
