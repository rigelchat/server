---
name: breaking-changes
description: Policy for proposing and applying breaking changes in rigel-server — always welcome when justified, provided they are explicitly flagged to the user.
---

# Breaking Changes Policy

1. **Always open to breaking changes**:
   - Propose and implement breaking changes (DB schema, REST/Gateway payloads or routes, `.env` config, internal refactors) whenever they yield a cleaner, more faithful, or safer design instead of awkward backwards-compatibility hacks.
2. **Mandatory explicit notice**:
   - Whenever a change introduces a breaking change (or auto-migrates an existing DB), **explicitly flag it** before or alongside the patch:
     - **What changes / breaks** (routes, payloads, DB tables/columns, `.env` keys, or Gateway events).
     - **Why** it is worthwhile.
     - **Action required** (e.g. automatic via `schema.rs` on boot, `.env` update, DB reset if needed, or client behavior impact).