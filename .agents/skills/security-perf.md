---
name: security-perf
description: Security, concurrency, and multi-thousand-user performance guardrails for rigel-server.
---

# Security & Performance Guardrails

Rigel targets **several thousand concurrent/registered users**. Every change must protect against N+1 stalls, deadlocks, memory blowups, and permission bypasses.

1. **Zero N+1 SQL & Bounded Batching**:
   - Never run SQL queries inside a `for` loop over members, channels, messages, or guilds. Batch with `QueryBuilder` (`IN (...)`) and hydrate via `HashMap`.
   - Chunk large `IN (...)` lookups and multi-row `INSERT ... ON DUPLICATE KEY UPDATE` writes (e.g. 500–1,000 rows per query) so `@everyone` or large guild updates never hit MySQL packet limits or stall the guild.
   - Always clamp `limit` parameters and paginate with indexed keys (`id_num` `VIRTUAL` columns or indexed timestamps).
2. **Transactions & Deadlock Prevention**:
   - Perform input validation, password hashing (`ARGON2_SEMAPHORE`), and read-only pre-checks **outside** SQL transactions.
   - Keep `tx` short. When locking multiple rows with `FOR UPDATE`, acquire locks in a deterministic order (`guild` → `channel`/`role`/`member`, or sorted IDs).
3. **Authorization, Hierarchy & IDOR Prevention**:
   - Never trust a client-supplied `channel_id`, `parent_id`, `role_id`, or `message_id` without verifying it belongs to the target `guild_id` / `channel_id`.
   - Enforce `VIEW_CHANNEL` + action permissions (`PermissionFlags`) on every route and Gateway dispatch (`dispatch_channel_with_ctx`), including private threads (`thread_members` or `MANAGE_THREADS`).
   - On moderation/role/member actions, enforce role hierarchy (`actor_highest > target_highest` unless guild owner) and never allow modifying the guild owner or `@everyone` position.
4. **Memory Bounds, Rate Limits & RAM Caching**:
   - Any in-memory structure (`rate_limit.rs`, `hub.rs`, future caches) must have explicit upper bounds and cleanup/eviction.
   - Classify new write, auth, or bulk routes in `src/rate_limit.rs` and expensive Gateway ops in `src/gateway/session.rs`.
   - Use `state.dispatch_to_sessions` / `dispatch_pre_serialized` for multi-session broadcasts.
   - Keep hot DB lookups (guild roles, overwrites, guild members) encapsulated inside `services/` so RAM caches in `AppState` can be plugged in cleanly with strict write invalidation.
5. **Untrusted Inputs & Future Media Uploads**:
   - Reject control characters (`reject_control_chars`), validate snowflakes (`validate_snowflake`), escape `LIKE` wildcards (`!`, `%`, `_`), and bound payload sizes.
   - Never trust client filenames or raw extensions for uploaded media; validate real file contents (decode/re-encode images server-side to strip EXIF/GPS and neutralize polyglots) and store under server-generated hashes/IDs.
