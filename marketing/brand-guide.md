# sqail Brand Guide

This guide is the single reference for how sqail looks, sounds, and shows up in public. It exists so that the portal, in-app UI, social posts, release notes, and future contributor assets all feel like the same product.

If you're about to ship something with the sqail name on it, skim this first.

## 1. Name & pronunciation

- **Written:** `sqail` — always lowercase in body copy and code. Never "SQaiL", "Sqail", or "SQAIL" in prose.
- **Pronounced:** *"snail."* The play on SQL + snail is deliberate: small, deliberate, carries its house with it.
- **Never:** "sequel", "ess-queue-ail", or "S-Q-A-I-L" spelled out.

Title case on the wordmark (logo) is a design choice made inside the logo asset; it does not change how we write the name in text.

## 2. Mascot

A snail. Friendly, unhurried, competent. Carries its shell = portable and self-contained. The mascot should feel confident, not cute or childish.

- Don't give the snail a face unless the asset is explicitly playful (error states, 404 pages).
- Don't use speed metaphors that contradict the snail joke — we own the paradox, we don't hide it.

## 3. Color palette

Extracted from `sqail.portal/src/index.css` so the portal, docs, and assets stay aligned.

### Dark theme (primary)

| Token | Hex | Usage |
|---|---|---|
| `bg-primary` | `#0F172A` | Page background |
| `bg-section` | `#1E293B` | Section / panel background |
| `bg-card` | `#273549` | Card hover / raised surface |
| `border` | `#334155` | Dividers, card borders |
| `text-primary` | `#F8FAFC` | Body copy, headings |
| `text-muted` | `#94A3B8` | Secondary copy |
| `text-dim` | `#64748B` | Tertiary / captions |

### Brand accents

| Token | Hex | Role |
|---|---|---|
| `brand-cyan` | `#38BDF8` | Primary accent — CTAs, active states, sqail wordmark gradient start |
| `brand-yellow` | `#FBBF24` | Secondary accent — AI features, highlights, wordmark gradient end |

### Rules

- **One accent per surface.** Cyan *or* yellow in a given card, not both. The gradient is reserved for the wordmark and hero headline.
- **Cyan owns "speed / open source / core product."** Yellow owns "intelligence / AI." Don't swap them.
- Never introduce a new brand color without updating this file and `index.css` together.

## 4. Typography

- **Portal + marketing:** system font stack — `-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`. Chosen for zero-payload load and native feel across OSes.
- **In-app:** inherits system font. Monaco editor uses its default monospace stack.
- **Headings:** bold (700), tight tracking (`tracking-tight`).
- **Body:** normal weight (400), relaxed leading for long-form.
- **Code / SQL:** monospace, never italicized.

If a future asset needs a display face, the first candidate is **Inter** (free, open, widely cached). Do not introduce a paid or obscure face without updating this guide.

## 5. Voice & tone

sqail's voice is the voice of a senior engineer writing to another engineer: terse, concrete, friendly, never smug.

### Do

- Use short sentences. Lead with the verb.
- Be specific: *"Opens in under a second"* beats *"Lightning-fast performance"*.
- Say what sqail *doesn't* do. Limits build trust.
- Credit other tools honestly when comparing. Don't punch down.
- Use lowercase *sqail* in body copy; capitalize other product names as they spell themselves.

### Don't

- Marketing clichés: "revolutionary", "game-changing", "next-generation", "unleash".
- Hype about AI that we can't demonstrate with a real query.
- Exclamation marks outside of genuine surprise (almost never).
- Emoji in product copy. OK sparingly in social / blog headers.
- Invented benchmarks. If we cite a number, it's measurable.

### Examples

| ❌ Avoid | ✅ Prefer |
|---|---|
| "Unleash the power of AI-driven SQL!" | "AI that knows your schema. Write SQL in plain English." |
| "Blazing fast, revolutionary editor" | "Opens before your terminal." |
| "Enterprise-grade privacy and security" | "No telemetry. Credentials stay on your machine." |
| "The only SQL client you'll ever need" | "Postgres, MySQL, SQLite, SQL Server — one editor." |

## 6. Messaging pillars

Every public artifact should map to at least one of these five. If it doesn't, it's probably not a sqail story.

| Pillar | Headline | One-line proof |
|---|---|---|
| **Fast** | "Opens before your terminal." | Under 20 MB, sub-second launch, native Tauri webview. |
| **Smart** | "AI that actually knows your schema." | Schema-aware autocomplete, schema context injection, streaming NL-to-SQL. |
| **Free** | "Open source. Forever." | MIT license, Codeberg + GitHub, no account required. |
| **Private** | "Your queries stay on your machine." | No telemetry, local encrypted storage, bring-your-own-AI-key. |
| **Universal** | "Postgres, MySQL, SQLite — one editor." | tauri-plugin-sql + tiberius, extensible driver model. |

## 7. Logo usage

Current mark lives at `src-tauri/icons/icon.png` (app) and `sqail.portal/public/icon.png` (portal favicon). A proper SVG + PNG export pipeline is a Phase 0 outstanding item.

### Rules

- Minimum clear space around the mark: one snail-shell width on every side.
- Minimum size: 24px for favicons, 32px for anywhere else.
- Never stretch, skew, rotate, recolor outside the palette, or reflow the mark.
- On dark backgrounds: full-color mark. On light: full-color mark. On busy photos: add a 12% dark scrim behind it, do not outline the mark itself.
- Never combine the mark with another wordmark without a separator and clear co-branding intent.

## 8. Screenshots

- **Theme:** dark mode first. Light mode screenshots only when the blog post is specifically about theming.
- **Window chrome:** include the title bar, no OS decorations around it — the app is the product, not the OS.
- **Sample data:** use the canned demo schemas. Never show real customer data, real API keys, or paths that reveal a contributor's home directory.
- **Resolution:** 2x DPI minimum. Export as PNG for the portal, GIF/MP4 for animated demos.
- **Focus:** one feature per shot. If you can't point at what the screenshot proves, retake it.

## 9. Social cards

Recommended sizes:

- **Open Graph / general:** 1200×630
- **X / Twitter header:** 1500×500
- **Square (Mastodon, Bluesky, LinkedIn post):** 1080×1080

All cards should include the sqail wordmark in a consistent corner and lead with the feature, not the brand.

## 10. What's out of scope (for now)

- Loyalty / account features, paid plans, feature gates — see `marketing-strategy.md` §9.
- Slogans we can't ship today. Don't market the roadmap.
- Any voice that positions sqail as an enterprise tool. That's a different product.

## 11. Changing this guide

The guide follows the product. If the palette, voice, or positioning changes:

1. Update this file first.
2. Update `sqail.portal/src/index.css` or relevant code in the same PR.
3. Call out the change in the PR description so reviewers can check downstream assets.

Drift between this guide and the shipped product is a bug — file it as such.
