---
name: breaking-changes
description: Policy for proposing and applying breaking changes in rigel-server — breaking changes are always welcome when justified, provided they are explicitly called out to the user.
---

# Breaking Changes Policy

1. **Always open to breaking changes**:
   - Do not hesitate to propose or implement breaking changes (DB schema changes, REST/Gateway payload or route changes, config/env changes, internal refactors) when they lead to a cleaner, more faithful, or more maintainable design instead of keeping awkward backwards-compatibility hacks.
2. **Mandatory explicit notice**:
   - Whenever a change introduces a breaking change, **explicitly flag it** in the response before or alongside the patch.
   - Clearly list:
     - **What breaks** (affected routes, payloads, DB tables/columns, config keys, or Gateway events).
     - **Why** the breaking change is worthwhile.
     - **Action required** (e.g., DB migration/reset, `.env` update, or client expectation impact).
