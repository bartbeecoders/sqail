# SQL Editor

## Test: editor is present and accepts input

**Preconditions:** main UI is visible

**Steps:**
1. Click inside the SQL editor area in the center of the app
2. Type "SELECT 1;"

**Expected:**
- The editor accepts text input
- SQL syntax highlighting is applied (keywords like SELECT are colored)

## Test: new editor tab

**Preconditions:** main UI is visible

**Steps:**
1. Find the tab bar above the editor
2. Click the "+" button to create a new tab (or use the new tab action)

**Expected:**
- A new editor tab appears
- The new tab is now the active tab
- The editor area shows an empty editor

## Test: switch between editor tabs

**Preconditions:** at least two editor tabs exist

**Steps:**
1. Type "SELECT 1;" in the current tab
2. Switch to the other tab
3. Type "SELECT 2;" in that tab
4. Switch back to the first tab

**Expected:**
- Each tab maintains its own content independently
- First tab still shows "SELECT 1;"
- Second tab still shows "SELECT 2;"

## Test: close editor tab

**Preconditions:** at least two editor tabs exist

**Steps:**
1. Find the close (X) button on one of the tabs
2. Click it

**Expected:**
- The tab is closed
- Another tab becomes active
- The editor shows that tab's content
