# Capturing Marketing Screenshots

End-to-end guide for producing the five portal screenshots. Follow top-to-bottom; the scripts handle everything mechanical.

## Prerequisites

Check once:

```bash
command -v sqlite3 && command -v xdotool && command -v gnome-screenshot && command -v magick
```

All four should exist. If any are missing, install them first.

## Step 1 — Seed the demo database

```bash
./scripts/seed-demo-db.sh
```

Writes `marketing/demo-data/sqail-demo.sqlite` (~44 KB: 15 authors, 30 books, 20 customers, 40 orders, 25 reviews). Safe to re-run any time — always overwrites the same file.

## Step 2 — Enter demo mode

**Quit sqail first.** The script refuses to run if sqail is still open, because the app writes `connections.json` on save and will clobber whatever you swap in.

```bash
./scripts/demo-mode.sh enter
```

This:

1. Backs up your real `connections.json`, `query_history.json`, `saved_queries.json`, `ai_history.json`, and `metadata.json` to `*.real-backup` files
2. Writes clean demo versions (three seeded connections, empty histories, empty metadata cache)
3. Creates a `.demo-mode-active` marker so exit knows what to restore

**Critical:** `ai_providers.json` is *not* swapped. Your real AI key stays configured so the AI panel works during the shot. Before taking the AI screenshot, verify no real key text is visible onscreen.

Check status any time:

```bash
./scripts/demo-mode.sh status
```

## Step 3 — Launch sqail and size the window

```bash
# Launch sqail however you normally do — the helper script or the installed app
./scripts/run.sh dev &
# Once the window is visible:
./scripts/screenshot-prep.sh
```

`screenshot-prep.sh` uses `xdotool` to resize the sqail window to exactly **1600×1000** at position (120, 120). Re-run it between shots if you accidentally resize.

## Step 4 — Capture the five shots

Create the raw capture folder once:

```bash
mkdir -p marketing/screenshots-raw
```

Then, for each of the five shots below, compose the state in sqail and run:

```bash
./scripts/screenshot-capture.sh <name>
```

This focuses the sqail window, waits for the compositor to settle, and captures it to `marketing/screenshots-raw/<name>.png`. Because sqail has `decorations: false` in `tauri.conf.json`, you get the app's own titlebar with no OS chrome around it — exactly what the brand guide asks for.

### 4a — `editor.png`

**Goal:** the core editor experience with a populated result grid.

1. Connect to `bookstore (sqlite)`.
2. Open a new tab, paste this:

   ```sql
   SELECT
     a.name         AS author,
     COUNT(o.id)    AS orders,
     ROUND(SUM(o.total_price), 2) AS revenue
   FROM authors a
   JOIN books b   ON b.author_id = a.id
   JOIN orders o  ON o.book_id   = b.id
   GROUP BY a.id
   ORDER BY revenue DESC
   LIMIT 10;
   ```

3. Run it (F5). Result grid populates with 10 rows.
4. Capture.

### 4b — `connections.png`

**Goal:** show that sqail is genuinely multi-driver, with a populated connection list.

1. Make sure the left sidebar is open and showing the connection list.
2. All three seeded connections should be visible: `bookstore (sqlite)`, `analytics-prod`, `legacy-cms`.
3. Expand `bookstore (sqlite)` so the table list (authors, books, customers, orders, reviews) is visible.
4. Capture.

### 4c — `ai.png`

**Goal:** AI sidebar mid-prompt, with the snail doing something useful.

1. Connect to `bookstore (sqlite)`.
2. Open the AI panel (`Ctrl+Shift+A`) or invoke the command palette (`Ctrl+K`).
3. Type a prompt like: *"Top 5 genres by total revenue in 2024, with average rating from reviews."*
4. Let the response stream. Capture mid-stream if you can — otherwise capture after the full response lands, making sure SQL is visible in the response.
5. **Before capturing, scroll the settings panel far away from view** so no API key or endpoint URL is visible.

### 4d — `split.png`

**Goal:** demonstrate split editor.

1. Open the same query file in two split panes (vertical split preferred).
2. Position cursors in different places — left pane showing the start of a query, right pane showing a JOIN further down.
3. Run a query in one side so the result grid renders.
4. Capture.

### 4e — `light.png`

**Goal:** prove light theme exists and is polished.

1. Switch to light theme via Settings or the theme toggle.
2. Pick the editor scene from 4a (same query, same result grid).
3. Capture.
4. Switch back to dark theme when done.

## Step 5 — Frame everything

```bash
./scripts/frame-all.sh
```

This runs `scripts/screenshot-frame.sh` against every raw in `marketing/screenshots-raw/` and drops 2880×1800 framed PNGs into `sqail.portal/public/screenshots/`. Dark background for `editor / connections / ai / split`; light background for `light`.

You can run it repeatedly as you capture — missing raws are skipped with a clear message. You can also frame one shot at a time: `./scripts/frame-all.sh editor`.

## Step 6 — Exit demo mode

```bash
./scripts/demo-mode.sh exit
```

Restores your real `connections.json`, histories, and metadata. Quit sqail first, same reason as step 2.

## Step 7 — Wire into the portal

Tell me the shots are in `sqail.portal/public/screenshots/` and I'll:

- Add a `Screenshots` section to the portal with a gallery
- Drop one into the Docs "Getting Started" panel
- Update `marketing-plan.md §2` to flip the screenshot checkbox

## Troubleshooting

**`demo-mode.sh enter` says "stale backup found"**
A previous `enter` crashed mid-way. Inspect `~/.local/share/dev.sqail/*.real-backup` manually — those *are* your real data. Copy them back to their live names if needed, then delete the backups.

**`screenshot-prep.sh` says "no sqail window found"**
The window title or class doesn't match. Check `xdotool search --name sqail` manually to see what's there, and adjust the script's `--name` pattern if sqail changed how it names its window.

**Framed screenshots look stretched or cropped**
The raw capture aspect ratio doesn't match what the framer expects. The framer fits the raw inside a 2640×1560 content area preserving aspect, so *any* aspect works — but if the raw is very wide or very tall, it will leave gradient bars on the short axis. Re-run `screenshot-prep.sh` before capturing if the window was manually resized.

**AI screenshot accidentally shows a real API key**
Stop, delete the raw, and retake. Never commit a raw that shows a real key, even briefly — git history is forever.
