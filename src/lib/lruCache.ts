/**
 * Tiny LRU cache built on top of `Map` insertion order.
 *
 * Used by the inline AI provider to memoise completions keyed by
 * (prefix hash, suffix hash, schema hash, model id) so identical
 * keystrokes don't re-issue an FIM request.
 */
export class LruCache<V> {
  private readonly map = new Map<string, V>();

  constructor(private readonly capacity: number) {
    if (capacity <= 0) throw new Error("LRU capacity must be > 0");
  }

  get(key: string): V | undefined {
    const v = this.map.get(key);
    if (v === undefined) return undefined;
    // Refresh recency by re-inserting.
    this.map.delete(key);
    this.map.set(key, v);
    return v;
  }

  set(key: string, value: V): void {
    if (this.map.has(key)) this.map.delete(key);
    this.map.set(key, value);
    if (this.map.size > this.capacity) {
      // Map iteration yields keys in insertion order — the first key is
      // the least-recently-inserted / -accessed.
      const oldest = this.map.keys().next().value;
      if (oldest !== undefined) this.map.delete(oldest);
    }
  }

  clear(): void {
    this.map.clear();
  }

  get size(): number {
    return this.map.size;
  }
}

/** djb2 — plenty for cache keys, trivially fast. */
export function hashString(s: string): string {
  let h = 5381;
  for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}
