# Sidebar Panels

## Test: switch between sidebar bottom tabs

**Preconditions:** main UI is visible, sidebar is expanded, a connection exists

**Steps:**
1. Find the bottom tab bar in the sidebar (Schema, History, Saved, Metadata icons)
2. Click the "History" tab (clock icon)
3. Observe the panel content changes
4. Click the "Saved" tab (bookmark icon)
5. Observe the panel content changes
6. Click the "Schema" tab (table icon)

**Expected:**
- Each tab switches the bottom panel content
- Schema tab shows database schema/tables (or empty state if not connected)
- History tab shows query history (or empty state)
- Saved tab shows saved queries (or empty state)

## Test: sidebar resize

**Preconditions:** main UI is visible, sidebar is expanded

**Steps:**
1. Find the right edge of the sidebar (the resize handle)
2. Drag it to the right to make the sidebar wider
3. Drag it to the left to make the sidebar narrower

**Expected:**
- The sidebar width changes as you drag
- The editor area adjusts to fill the remaining space
- The sidebar does not go narrower than a minimum width or wider than a maximum width
