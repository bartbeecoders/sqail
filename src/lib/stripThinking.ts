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
