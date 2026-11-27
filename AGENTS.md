# Rigel Server — Project Instructions

Lightweight, self-hosted chat backend in Rust (`edition = 2024`, Axum 0.8, SQLx 0.8, **MySQL >= 8.0**, Tokio, WebSocket Gateway), designed to scale to several thousand users.

## Protocol & Product Scope

- **Source of truth**: **Fluxer** HTTP/Gateway specification first; keep secondary compatibility with Discord/Spacebar where structs align naturally.
- **Included / Planned**: Guilds, channels, forums/threads, messages (replies, mentions), discovery, moderation (kick/ban/audit logs), local dynamic media storage (avatars, banners, icons, emojis, GIFs), simple `.env` premium perk toggle, future bot support (`Bot ` token), and future admin CLI/TUI subcommands (e.g. `rigel-server update`).
- **Out of Scope**: DMs / Group DMs, Voice / WebRTC, Stripe / billing, third-party OAuth2, quests, web admin panel, protobuf.
- **Configuration**: Instance policy lives in `.env` (`src/config.rs`); protocol limits live in `src/constants/limits.rs`.

## Architecture & Sizing

Keep `1 file ≈ 1 clear responsibility`. Split growing domains into sub-modules (target ~400–600 lines max for readability), without over-fragmenting trivial logic into micro-files.

- `src/routes/` — Axum handlers using extractors (`AuthUser`, `FluxerFeatures`, `AppJson`, `AppError`). **Zero SQL.**
- `src/services/` — Business logic & typed SQL (`sqlx::query_as` / `QueryBuilder`), split by domain (`channel/`, `guild/`, `role/`, `thread/`, `user/`, plus focused single-file services and `queries.rs`). **Zero raw `StatusCode`** — always use semantic `AppError::*` constructors from `src/error.rs`.
- `src/models/` — Typed request/response/DB structs. Avoid raw `serde_json::Value` / `json!()` blobs where a struct fits.
- `src/db/` — Pool setup (`mod.rs`) and MySQL 8.0+ idempotent schema/migrations (`schema.rs`).
- `src/gateway/` — WebSocket lifecycle, rate limiting, and 1 handler file per opcode under `src/gateway/handlers/`.
- `src/rate_limit.rs` — Bounded in-memory HTTP rate limiter middleware.
- `src/constants/` — Domain constants (`channel`, `flags`, `gateway`, `guild`, `invite`, `limits`, `message`, `user`).
- `src/utils/` — Shared utilities (`assets`, `snowflake`, `token`, `validation`).

## Coding Conventions

- **Naming**: `*Row` for DB row structs (with `SELECT_COLS` / `SELECT_FROM`), `*Request` / `*Query` / `*Response` for REST, `Partial*` / `*Event` for Gateway.
- **Partial Updates**: Use `Option<Option<T>>` with `deserialize_double_option` on nullable `PATCH` fields (`None` = omitted vs `Some(None)` = `null`).
- **Capabilities**: Respect `FluxerFeatures` / `SessionFlags` only where the Fluxer spec gates features (e.g. `channel_threads`).
- **Safety**: **No `.unwrap()` or `.expect()`** outside `#[cfg(test)]` modules (except fatal startup setup in `main.rs`).
- **Language**: Logs, error messages, code comments, and commit messages in **English only**; keep comments minimalist (only explain non-obvious invariants).
- **Tests**: Write unit tests (`#[cfg(test)]`) for pure logic and keep `tests/api_smoke.rs` working without requiring a live MySQL server.
- **Verification**: Always finish code changes with `cargo check && cargo clippy --all-targets -- -D warnings && cargo test`.

## Skills

- `.agents/skills/breaking-changes.md` — Policy for DB, API, and config breaking changes.
- `.agents/skills/safe-patch.md` — Atomic patch generation rules (Repomix + shell/Python workflow).
- `.agents/skills/security-perf.md` — Security, concurrency, and multi-thousand-user scale rules.
- `.agents/skills/fluxer-feature.md` — End-to-end checklist for adding or updating Fluxer endpoints/events.