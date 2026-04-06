# Settings

## Test: open settings modal

**Preconditions:** main UI is visible

**Steps:**
1. Find and click the settings button (gear icon) in the toolbar or sidebar

**Expected:**
- A settings modal/dialog appears
- It has tabs: General, AI, Shortcuts, Snippets, About

## Test: navigate settings tabs

**Preconditions:** settings modal is open

**Steps:**
1. Click on the "Shortcuts" tab
2. Observe the content changes to show keyboard shortcuts
3. Click on the "Snippets" tab
4. Observe the content changes to show SQL snippets
5. Click on the "About" tab
6. Observe the content shows app info
7. Click on the "General" tab

**Expected:**
- Each tab shows its respective content
- Tab switching is instant with no errors

## Test: close settings modal

**Preconditions:** settings modal is open

**Steps:**
1. Click the close (X) button on the settings modal

**Expected:**
- The settings modal closes
- The main app UI is visible and interactive again
