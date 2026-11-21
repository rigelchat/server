---
name: safe-patch
description: Use when generating file edits or bash patch commands for rigel-server so modifications are atomic, minimal, and never corrupt or truncate files.
---

# Safe Patching Rules

1. **New or heavily rewritten files**: output the file directly in a fenced code block (keep files small per the 1-file-1-responsibility rule; avoid giant `cat`/`echo` heredocs).
2. **Targeted edits**:
   - Prefer `git apply --check <<'EOF' && git apply <<'EOF'` when exact current file contents are known from the provided Repomix.
   - When using `python3`:
     - **NEVER** write `open(p, "w").write(open(p).read())` (Python evaluates `open(p, "w")` first and truncates the file to 0 bytes before `read()` runs).
     - Always use `pathlib.Path(p).read_text()` into a variable, apply and `assert` all replacements **in memory first**, and only call `write_text()` after every check passes.
3. **Verification**: always end code changes with `cargo check` (fix any new warnings or errors before moving on).
4. **Git hygiene**: never stage or commit `.env`, secrets, `/target/`, or `/static/emoji/`.