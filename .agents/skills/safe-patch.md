---
name: safe-patch
description: Rules for generating safe, atomic, copy-paste-ready patches (Repomix + terminal workflow) without corrupting or truncating files.
---

# Safe Patching Rules

This repo is primarily worked on via **Repomix context + terminal patches** (or direct editing tools if available). When generating patches for the terminal:

1. **Split large changes into small, self-contained blocks**:
   - Keep each block focused (~100–350 lines max, grouped by layer: constants/DB/errors, models, services, routes).
   - Never generate one giant script touching 15+ files at once.
2. **Markdown fences & readability**:
   - Always use 4 backticks (````) for outer code blocks so embedded triple-backtick blocks never break rendering.
3. **Targeted edits (Python only, never `git apply`)**:
   - Never output multi-hunk unified diffs (`@@` line counts are fragile).
   - Use a unique heredoc delimiter (`<<'__SAFE_PATCH_EOF__'`, never `EOF` or `PY`), or pick a distinct delimiter if the patch text itself mentions `__SAFE_PATCH_EOF__`.
   - Use raw triple-quoted strings (`r'''...'''`) when Rust snippets contain backslashes or raw strings (`r#"..."#`).
   - **NEVER** open a file for writing before reading it in full (`open(p, "w").write(open(p).read())` truncates to 0 bytes).
4. **All-or-nothing atomicity (mandatory template)**:
   - **Phase 1 (in-memory only)**: read every target file, assert `text.count(old) == 1` (or exact `re.subn` match count) for every replacement, and stage all edits in memory. If any assertion fails, **zero files are touched on disk**.
   - **Phase 2 (backup + rollback)**: snapshot original bytes before writing and restore every modified/created/deleted file in `except Exception:` if anything fails.
5. **Verification & Git hygiene**:
   - End code changes with: `cargo check && cargo clippy --all-targets -- -D warnings && cargo test`.
   - Never stage or commit `.env`, secrets, `/target/`, `/static/emoji/`, or runtime media upload directories.
   - Never modify or commit unrelated files (such as `.github/workflows/release.yml`) unless explicitly requested.

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