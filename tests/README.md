# UI Test Prompts

AI-driven UI test scenarios for sqail. Each `.test.md` file contains test cases
written as natural-language prompts. Claude Code executes these against the
running app using browser automation tools.

## How to run

1. Start the app: `pnpm tauri dev`
2. Ask Claude Code: "run the UI tests" (or a specific test file)

Claude Code will open the app in Chrome, follow each step, and report
pass/fail with screenshots on failure.

## Writing tests

Each test file follows this format:

```markdown
# Test Suite Name

## Test: descriptive name

**Preconditions:** any required state before the test

**Steps:**
1. natural language instruction
2. another instruction
...

**Expected:** what should be true after the steps
```

Keep steps atomic and observable. Avoid referencing CSS classes or DOM internals
-- describe what a user would see and do.
