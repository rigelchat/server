---
name: fluxer-feature
description: End-to-end checklist for implementing or updating a Fluxer REST/Gateway feature in rigel-server.
---

# Fluxer Feature Checklist

Follow this order whenever adding or modifying a protocol feature:

1. **Constants (`src/constants/`)**:
   - Define protocol limits in `limits.rs`, bitflags in `flags.rs`, and domain enums/values in their domain module.
2. **Database (`src/db/schema.rs`)**:
   - Target **MySQL >= 8.0** (`utf8mb4`).
   - Add tables via `CREATE TABLE IF NOT EXISTS` and keep existing DBs working automatically on startup using `INFORMATION_SCHEMA` helpers (`ensure_column` / `ensure_index`) — never MariaDB-only syntax like `ADD COLUMN IF NOT EXISTS`.
   - Add `id_num` `VIRTUAL` columns + indexes for any snowflake-paginated table.
3. **Errors (`src/error.rs`)**:
   - Add semantic `ApiErrorCode` / `ValidationCode` variants and `AppError::*` constructors; never import `StatusCode` in `src/services/`.
4. **Models (`src/models/`)**:
   - Add `*Row` (with `SELECT_COLS` / `SELECT_FROM`), `*Request` / `*Query` / `*Response`, and Gateway `*Event` structs.
   - Use `Option<Option<T>>` + `deserialize_double_option` for nullable `PATCH` fields.
5. **Services (`src/services/`)**:
   - Validate inputs → check membership, permissions, and hierarchy → run atomic DB mutations → emit Gateway events.
6. **Gateway & Visibility (`src/gateway/`)**:
   - Dispatch events only to authorized sessions (`dispatch_channel_with_ctx`, `dispatch_guild`, `dispatch_user`), include `READY` updates if stateful, and call `lazy_request::refresh_guild_member_lists` when roles, nicknames, members, or channel overwrites change.
7. **Routes & Rate Limiting (`src/routes/`, `src/rate_limit.rs`)**:
   - Keep routes free of SQL; use `AuthUser`, `FluxerFeatures` (only where Fluxer spec gates capabilities), and `AppJson`. Ensure `rate_limit.rs` classifies the route properly.
8. **Tests (`#[cfg(test)]` & `tests/api_smoke.rs`)**:
   - Add unit tests for pure parsing/validation/permission logic and DB-free smoke tests for auth rejection, validation, and rate limits.
