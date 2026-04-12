# sqail Marketing Plan

Tactical execution plan derived from `marketing-strategy.md`. This is the *what, when, and how* — specific activities, owners, and deliverables. Update this as the plan runs.

## 1. Phases Overview

| Phase | Focus | Exit criteria |
|---|---|---|
| **0. Pre-launch** | Portal, branding, docs, first stable binary | sqail.io live, v0.4 downloadable on all three platforms, README polished |
| **1. Soft launch** | Friends, early feedback, fix sharp edges | ~50 real users, major bugs triaged, install flow validated on each OS |
| **2. Public launch** | HN / Reddit / ProductHunt moment | Launch posts live, traffic handled, press kit shipped |
| **3. Growth** | Sustained content, community, iterative releases | Monthly release cadence, active issue tracker, returning visitors to portal |
| **4. Maturity** | Ecosystem, integrations, contributor base | Third-party drivers, external contributions, featured in roundups |

## 2. Pre-launch Checklist (Phase 0)

### Portal (sqail.io)

- [x] Hero section with tagline, snail logo, primary download CTA (OS auto-detect)
- [x] Feature grid keyed to the five messaging pillars (Fast, Smart, Free, Private, Universal)
- [x] Screenshots: editor, connections, AI sidebar, split editor, light mode — wired into portal `Screenshots` gallery + Docs Getting Started
- [ ] 30-60s animated demo GIF (NL-to-SQL in action) — *needs user capture*
- [x] Downloads page with per-OS binaries, SHA256 checksums, version + build date
- [x] Changelog page (mirror of RELEASES.md)
- [x] "Why sqail" comparison table (vs. DBeaver, DataGrip, TablePlus, Beekeeper)
- [x] Docs section: Getting Started, Connections, AI Setup, Keyboard Shortcuts
- [x] Footer: Codeberg, GitHub, License, Privacy (no telemetry statement)

### Branding assets

- [ ] Finalized snail logo (SVG, PNG @1x/@2x, favicon, OG image) — *raster icons present, SVG pipeline outstanding*
- [x] Color palette documented (primary, accent, neutral grays, dark/light variants) — see `brand-guide.md`
- [x] Typography pairing (portal + in-app consistency) — see `brand-guide.md`
- [x] Screenshot template — `scripts/screenshot-frame.sh`, documented in `brand-guide.md` §8
- [~] Press kit text drafted in `marketing/press-kit/` — *final ZIP pending logo + screenshots*
- [ ] Social banners: 1500×500 (X), 1200×630 (OG), 1080×1080 (square)

### Product readiness

- [x] v0.4.0 stable across Linux / macOS / Windows
- [x] First-run experience: splash screen, onboarding to add first connection
- [x] AI provider setup flow: friction-free "paste key, pick model"
- [ ] Crash-free rate acceptable on all three platforms
- [x] README with 3-paragraph elevator pitch + install + first-query GIF — *pitch + install done, GIF pending*
- [x] LICENSE file at root (already present)
- [x] CONTRIBUTING.md for future OSS contributors

## 3. Launch Sequence (Phase 2)

### T-14 days — Warmup

- Tease on personal social accounts ("building something small and fast")
- Publish "Why I'm building sqail" blog post on the portal
- Line up 5-10 beta users from network for pre-launch feedback
- Write and sit on the HN/Reddit/PH copy; let it breathe, edit with fresh eyes

### T-7 days — Final rehearsal

- Dry-run downloads on each OS from a fresh VM
- Verify all links in portal, README, press kit
- Pre-schedule release binaries with deterministic filenames
- Confirm domain, SSL, portal deployment pipeline all green
- Draft release notes for the launch version

### T-0 — Launch day

**Morning (US East Coast 08:00-10:00 for HN prime time):**
- Publish Show HN: *"Show HN: sqail — a 20MB open-source SQL editor with native AI"*
- Post to r/programming, r/Database, r/rust, r/tauri (space out by 15-30 min; don't spam)
- ProductHunt launch (schedule for 00:01 PT the day of)
- X/Bluesky/Mastodon thread with demo GIF
- Dev.to cross-post of the launch blog

**All day:**
- Actively respond to every HN/Reddit comment within 30 minutes
- Fix any install-blocking bugs in real time; ship a hotfix if needed
- Bookmark and thank every mention, star, and useful critique

**Evening:**
- Lobste.rs post (if you have an invite; wait for organic signal otherwise)
- Post-launch retro note: what worked, what to fix tomorrow

### T+1 to T+7 — Followup

- Reply to late comments and issues
- Ship at least one bugfix release addressing launch feedback
- Write a "Launch day, by the numbers" blog post for transparency
- Email/DM thank-yous to anyone who amplified

## 4. Ongoing Content Calendar (Phase 3+)

Target: **1 meaningful public artifact per week**, rotating through the types below.

| Week slot | Content type | Example |
|---|---|---|
| Week 1 | Release notes + demo GIF | "v0.5 — inline cell editing, streaming AI responses" |
| Week 2 | Technical deep-dive | "How sqail injects schema context into AI prompts" |
| Week 3 | Migration / comparison guide | "Switching from DBeaver to sqail in 5 minutes" |
| Week 4 | Community / transparency | "One month of sqail: stars, downloads, lessons" |

### Release cadence

- **Patch releases** (bugfixes): as needed, same week as discovery
- **Minor releases** (features): monthly, bundled with a demo GIF + changelog post
- **Major releases**: when scope justifies a new launch moment

## 5. Channel-Specific Tactics

### Hacker News
- **Post format:** *Show HN: [product] — [one-sentence value prop]*
- **Timing:** Tuesday-Thursday, 08:00-10:00 ET
- **Body:** Problem, why existing tools don't solve it, what sqail does, what's next. Link to portal + Codeberg.
- **Engagement rule:** first-author replies within 30 min for the first 4 hours

### Reddit
- **r/programming** — launch + major releases only (no spam)
- **r/Database, r/SQL** — feature highlights, tutorials, migration guides
- **r/rust, r/tauri** — engineering posts (binary size, performance, Tauri lessons)
- **r/opensource, r/selfhosted** — "no telemetry, free forever" angle
- **r/dataengineering** — AI-for-SQL angle, analyst-facing framing

### X / Bluesky / Mastodon
- **Cadence:** 2-3 posts/week, not more
- **Content:** short demos, release notes, interesting bugs, engineering notes
- **Engagement:** reply to dev-tooling conversations, don't chase unrelated virality

### Blog (on portal)
- **First posts (in order):**
  1. "Why sqail — another SQL client, really?"
  2. "Shipping a 20 MB SQL editor with Tauri"
  3. "Schema-aware AI: making NL-to-SQL actually work"
  4. "From idea to launch day — sqail's first month by the numbers"

### Awesome lists & directories
- Submit to `awesome-tauri`, `awesome-rust`, `awesome-sql`, `awesome-selfhosted`, `awesome-opensource-ai-tools`
- Add to AlternativeTo entries for DBeaver, DataGrip, TablePlus, Beekeeper Studio

### Community
- **Discussions:** GitHub Discussions as the primary Q&A venue (Codeberg issues for bugs, Codeberg discussions if enabled)
- **Chat:** Matrix room (`#sqail:matrix.org`) — lower barrier than Discord, fits open-source ethos
- **Code of conduct:** adopt Contributor Covenant; enforce from day one

## 6. Asset Production Pipeline

Use `image-gen` / xAI-image MCP for brand illustrations (per `Vibecoding/marketing.md` conventions). Every release cycle:

1. One hero GIF showing the headline feature of the release
2. One blog post with inline screenshots
3. One social-card variant (1200×630) matching the blog hero
4. Updated press kit if a major version

## 7. Measurement & Feedback Loops

### Weekly review (Mondays, 30 min)

- Download counts per OS, per version
- Portal traffic + top referrers
- Codeberg/GitHub stars, issues opened/closed
- Social mentions (manual scan)
- Note one thing that worked, one thing to change

### Monthly review (first Monday of the month)

- Progress vs. 90-day targets (see strategy §8)
- Top issues by frequency → inform roadmap
- Retire / rewrite any content that underperformed
- Plan the next month's release theme

### Quarterly review

- Revisit strategy document; update positioning or targets if reality diverges
- Publish a transparent "state of sqail" post

## 8. Budget

sqail is bootstrapped and open source. Default assumption: **zero cash spend** on paid marketing in the first year.

| Category | Estimated spend (year 1) | Notes |
|---|---|---|
| Domain (sqail.io) | low annual fee | already owned |
| VPS (k3s portal hosting) | already provisioned | shared with other projects |
| Code signing certificates | moderate one-time + renewal | **optional, delayed** — only if Windows/macOS trust warnings become a friction point |
| Design tooling | $0 | use existing MCP image generation + open-source tools |
| Paid ads | $0 | we compete on content quality and organic reach |

If a paid push is ever justified, reconsider *after* reaching the 90-day targets organically — not before.

## 9. Ownership

Today, all marketing roles are held by the maintainer. This section exists so future contributors can take over cleanly:

| Role | Responsibility |
|---|---|
| **Maintainer** | Strategy, launch posts, release notes, comment replies |
| **Community lead** (future) | Discussions, Matrix room, contributor onboarding |
| **Content writer** (future) | Blog post cadence, migration guides |
| **Designer** (future) | Logo refinements, screenshot consistency, social cards |

## 10. Done Is Better Than Perfect

Two rules for anything in this plan:

1. **If a task has been on the list for more than two weeks, either do it or delete it.**
2. **Ship first, polish in public.** Every week without visible progress is a week of lost momentum.
