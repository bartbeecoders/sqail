/**
 * Remove `<think>...</think>` reasoning blocks from model output.
 *
 * Reasoning-capable models (DeepSeek-R1, QwQ, …) sometimes emit their
 * chain-of-thought in `<think>` tags. For ghost-text inline completion
 * we never want that visible — it's both noise and a leaky abstraction.
 *
 * Handles:
 *  - complete `<think>...</think>` blocks anywhere in the text
 *  - an unterminated `<think>` that's still streaming — everything from
 *    it onward is hidden until the closing tag arrives.
 *
 * Pure function — safe to call on every streamed chunk's accumulator.
 */
export function stripThinkingBlocks(text: string): string {
  let result = text.replace(/<think>[\s\S]*?<\/think>/g, "");
  const openIdx = result.indexOf("<think>");
  if (openIdx !== -1) {
    result = result.slice(0, openIdx);
  }
  return result.trimStart();
}

/**
 * Unwrap a response that is entirely one fenced code block
 * (e.g. ```sql\nSELECT ...\n```). Local instruct models (Qwen-Coder,
 * DeepSeek-Coder-V2) often do this even when the prompt says not to.
 *
 * Only strips when the fence *is* the whole response — mixed prose with
 * embedded fenced blocks (explain / document flows) is left untouched.
 * Handles the mid-stream case where the opening fence has arrived but
 * the closing fence has not.
 */
export function stripWrappingCodeFence(text: string): string {
  const trimmed = text.trim();
  if (!trimmed.startsWith("```")) return text;

  const closed = trimmed.match(/^```[a-zA-Z0-9_+-]*\n?([\s\S]*?)\n?```\s*$/);
  if (closed) return closed[1];

  const afterOpen = trimmed.replace(/^```[a-zA-Z0-9_+-]*\n?/, "");
  if (!afterOpen.includes("```")) return afterOpen;

  return text;
}
