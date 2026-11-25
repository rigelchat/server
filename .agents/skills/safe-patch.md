---
name: safe-patch
description: Use when generating file edits or bash patch commands for rigel-server so modifications are split into small blocks, atomic, all-or-nothing, and never corrupt or truncate files.
---

# Safe Patching Rules

1. **Split large changes into small, self-contained blocks**:
   - Never generate one giant script touching 15+ files at once (split by layer/domain into small blocks of ~100–350 lines max: e.g., constants, schema/errors, models, services, routes).
   - Whenever possible, make each block independently compilable or clearly order blocks so the final block compiles cleanly.
2. **New or heavily rewritten files & Markdown fences**:
   - Keep files small per the 1-file-1-responsibility rule.
   - Output standalone files in fenced code blocks or write them via `write_new(...)` inside a small Python block.
   - Always use 4 backticks (````) for outer code blocks in responses so embedded triple-backtick blocks (like Markdown or examples) never break rendering.
3. **Targeted edits (use Python, never `git apply`)**:
   - Do **not** generate multi-hunk `git apply` / unified diffs (line counts in `@@` headers are error-prone).
   - Always use a unique heredoc delimiter like `<<'__SAFE_PATCH_EOF__'` (never `EOF` or `PY`, which can appear inside snippets or markdown examples).
   - Use raw triple-quoted strings (`r'''...'''`) when Rust snippets contain backslashes (`\u{...}`, `\0`) or raw strings (`r#"..."#`).
   - **NEVER** write `open(p, "w").write(open(p).read())` (truncates the file to 0 bytes before `read()` runs).
4. **All-or-nothing atomicity (mandatory Python template)**:
   - **Phase 1 (in-memory only)**: read every target file into memory, assert `text.count(old) == 1` for each replacement, and stage all edits in memory. If **any** assertion fails, **zero files are modified on disk**.
   - **Phase 2 (atomic flush + rollback)**: snapshot original bytes before writing and restore every modified/created/deleted path in `except Exception:` if an I/O error occurs.
5. **Verification & Git hygiene**:
   - End code changes with `cargo check` (and `cargo clippy --all-targets -- -D warnings` + `cargo test` when ready).
   - Never stage or commit `.env`, secrets, `/target/`, or `/static/emoji/`.

## Safe Python Patch Template

```bash
python3 - <<'__SAFE_PATCH_EOF__'
from pathlib import Path

ROOT = Path(".")
staged: dict[Path, str] = {}
to_delete: set[Path] = set()

def load(rel: str) -> str:
    p = ROOT / rel
    if p not in staged:
        staged[p] = p.read_text(encoding="utf-8")
    return staged[p]

def write_new(rel: str, content: str) -> None:
    staged[ROOT / rel] = content.lstrip("\n")

def patch(rel: str, replacements: list[tuple[str, str]]) -> None:
    text = load(rel)
    for old, new in replacements:
        count = text.count(old)
        assert count == 1, f"{rel}: expected 1 match (got {count}) for {old[:80]!r}"
        text = text.replace(old, new, 1)
    staged[ROOT / rel] = text

# --- Phase 1: stage & assert everything in memory (0 disk writes yet) ---
# patch("src/example.rs", [("old", "new")])

# --- Phase 2: backup + flush with rollback ---
backups: dict[Path, bytes | None] = {
    p: (p.read_bytes() if p.exists() else None)
    for p in set(staged) | to_delete
}
touched: list[Path] = []
try:
    for p, content in staged.items():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(content, encoding="utf-8")
        touched.append(p)
    for p in to_delete:
        if p.exists():
            p.unlink()
            touched.append(p)
except Exception:
    for p in reversed(touched):
        prev = backups.get(p)
        if prev is None:
            p.unlink(missing_ok=True)
        else:
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_bytes(prev)
    raise

for p in sorted(touched):
    print(f"updated {p}")
__SAFE_PATCH_EOF__
```