# App Launch & Layout

## Test: splash screen displays and completes

**Preconditions:** app freshly opened

**Steps:**
1. Observe the app window after launch

**Expected:**
- A splash screen is visible briefly
- After the splash screen completes, the main UI is shown

## Test: main layout elements are present

**Preconditions:** splash screen has completed

**Steps:**
1. Look at the overall app layout

**Expected:**
- A custom title bar is visible at the top with the app name "SQaiL" and window controls (minimize, maximize, close)
- A sidebar is visible on the left
- The main editor area is visible in the center
- A toolbar is visible above the editor area

## Test: sidebar can be collapsed and expanded

**Preconditions:** main UI is visible

**Steps:**
1. Find the sidebar collapse/expand toggle button
2. Click it to collapse the sidebar
3. Observe the sidebar is now collapsed (narrow, icons only)
4. Click the toggle button again to expand

**Expected:**
- Sidebar collapses to a narrow strip
- Sidebar expands back to its original width
