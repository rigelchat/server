use sqlx::mysql::MySqlPool;

pub(super) async fn apply(pool: &MySqlPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS users (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            created_at BIGINT UNSIGNED NOT NULL,
            bot BOOLEAN NOT NULL DEFAULT FALSE,
            public_flags INT UNSIGNED NOT NULL DEFAULT 0,
            username VARCHAR(32) COLLATE utf8mb4_unicode_ci NOT NULL UNIQUE,
            email VARCHAR(255) COLLATE utf8mb4_unicode_ci DEFAULT NULL UNIQUE,
            verified BOOLEAN NOT NULL DEFAULT TRUE,
            password_hash VARCHAR(255) DEFAULT NULL,
            global_name VARCHAR(32) DEFAULT NULL,
            discriminator VARCHAR(4) NOT NULL DEFAULT '0000',
            avatar VARCHAR(255) DEFAULT NULL,
            banner VARCHAR(255) DEFAULT NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS instance_locks (
            name VARCHAR(64) NOT NULL PRIMARY KEY
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("INSERT IGNORE INTO instance_locks (name) VALUES ('bootstrap_staff')")
        .execute(pool)
        .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_profiles (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            bio VARCHAR(320) DEFAULT NULL,
            pronouns VARCHAR(40) DEFAULT NULL,
            accent_color INT UNSIGNED DEFAULT NULL,
            theme_color_primary INT UNSIGNED DEFAULT NULL,
            theme_color_secondary INT UNSIGNED DEFAULT NULL,
            FOREIGN KEY (id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            status VARCHAR(16) NOT NULL DEFAULT 'online',
            afk_timeout INT UNSIGNED NOT NULL DEFAULT 600,
            locale VARCHAR(16) NOT NULL DEFAULT 'en-US',
            theme VARCHAR(16) NOT NULL DEFAULT 'dark',
            background_gradient_preset VARCHAR(32) DEFAULT NULL,
            developer_mode BOOLEAN NOT NULL DEFAULT FALSE,
            synced_preferences TEXT DEFAULT NULL,
            extra_settings TEXT DEFAULT NULL,
            FOREIGN KEY (id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_sessions (
            id VARCHAR(36) NOT NULL PRIMARY KEY,
            created_at BIGINT UNSIGNED NOT NULL,
            last_used_at BIGINT UNSIGNED DEFAULT NULL,
            user_id VARCHAR(20) NOT NULL,
            os TEXT DEFAULT NULL,
            platform TEXT DEFAULT NULL,
            country_code VARCHAR(2) DEFAULT NULL,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS guilds (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            created_at BIGINT UNSIGNED NOT NULL,
            owner_id VARCHAR(20) NOT NULL,
            name VARCHAR(100) NOT NULL,
            icon VARCHAR(255) DEFAULT NULL,
            banner VARCHAR(255) DEFAULT NULL,
            description VARCHAR(300) DEFAULT NULL,
            rules_channel_id VARCHAR(20) DEFAULT NULL,
            vanity_url_code VARCHAR(20) DEFAULT NULL UNIQUE,
            afk_channel_id VARCHAR(20) DEFAULT NULL,
            afk_timeout INT UNSIGNED NOT NULL DEFAULT 300,
            system_channel_id VARCHAR(20) DEFAULT NULL,
            system_channel_flags INT UNSIGNED NOT NULL DEFAULT 0,
            discoverable BOOLEAN NOT NULL DEFAULT FALSE,
            verification_level TINYINT UNSIGNED NOT NULL DEFAULT 0,
            default_message_notifications TINYINT UNSIGNED NOT NULL DEFAULT 0,
            explicit_content_filter TINYINT UNSIGNED NOT NULL DEFAULT 0,
            nsfw BOOLEAN NOT NULL DEFAULT FALSE,
            INDEX idx_guilds_discoverable (discoverable)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS roles (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            guild_id VARCHAR(20) NOT NULL,
            position INT UNSIGNED NOT NULL,
            name VARCHAR(100) NOT NULL,
            color INT UNSIGNED NOT NULL DEFAULT 0,
            hoist_position INT DEFAULT NULL,
            icon VARCHAR(255) DEFAULT NULL,
            unicode_emoji VARCHAR(32) DEFAULT NULL,
            hoist BOOLEAN NOT NULL DEFAULT FALSE,
            mentionable BOOLEAN NOT NULL DEFAULT FALSE,
            permissions BIGINT UNSIGNED NOT NULL DEFAULT 0,
            INDEX idx_roles_guild_pos (guild_id, position),
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS channels (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            type INT UNSIGNED NOT NULL,
            position INT UNSIGNED NOT NULL,
            guild_id VARCHAR(20) NOT NULL,
            parent_id VARCHAR(20) DEFAULT NULL,
            name TEXT NOT NULL,
            last_message_id VARCHAR(20) DEFAULT NULL,
            last_pin_timestamp BIGINT UNSIGNED DEFAULT NULL,
            topic TEXT DEFAULT NULL,
            url VARCHAR(2048) DEFAULT NULL,
            nsfw BOOLEAN NOT NULL DEFAULT FALSE,
            nsfw_override BOOLEAN DEFAULT NULL,
            content_warning_level TINYINT UNSIGNED NOT NULL DEFAULT 0,
            content_warning_text VARCHAR(200) DEFAULT NULL,
            rate_limit_per_user INT UNSIGNED NOT NULL DEFAULT 0,
            flags INT UNSIGNED NOT NULL DEFAULT 0,
            default_auto_archive_duration INT UNSIGNED DEFAULT NULL,
            default_thread_rate_limit_per_user INT UNSIGNED NOT NULL DEFAULT 0,
            default_reaction_emoji_id VARCHAR(20) DEFAULT NULL,
            default_reaction_emoji_name VARCHAR(64) DEFAULT NULL,
            default_sort_order TINYINT UNSIGNED DEFAULT NULL,
            default_forum_layout TINYINT UNSIGNED NOT NULL DEFAULT 0,
            default_tag_setting VARCHAR(16) NOT NULL DEFAULT 'match_some',
            bitrate INT UNSIGNED DEFAULT NULL,
            user_limit INT UNSIGNED DEFAULT NULL,
            voice_connection_limit INT UNSIGNED DEFAULT NULL,
            INDEX idx_channels_guild_pos (guild_id, position),
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE,
            FOREIGN KEY (parent_id) REFERENCES channels(id) ON DELETE SET NULL
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS forum_tags (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            channel_id VARCHAR(20) NOT NULL,
            position INT UNSIGNED NOT NULL DEFAULT 0,
            name VARCHAR(50) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL,
            moderated BOOLEAN NOT NULL DEFAULT FALSE,
            emoji_id VARCHAR(20) DEFAULT NULL,
            emoji_name VARCHAR(64) DEFAULT NULL,
            UNIQUE KEY uq_forum_tags_channel_name (channel_id, name),
            INDEX idx_forum_tags_channel_pos (channel_id, position),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS channel_threads (
            channel_id VARCHAR(20) NOT NULL PRIMARY KEY,
            guild_id VARCHAR(20) NOT NULL,
            parent_id VARCHAR(20) NOT NULL,
            owner_id VARCHAR(20) NOT NULL,
            started_from_message BOOLEAN NOT NULL DEFAULT FALSE,
            archived BOOLEAN NOT NULL DEFAULT FALSE,
            auto_archive_duration INT UNSIGNED NOT NULL DEFAULT 4320,
            archive_timestamp BIGINT UNSIGNED NOT NULL,
            locked BOOLEAN NOT NULL DEFAULT FALSE,
            invitable BOOLEAN DEFAULT NULL,
            create_timestamp BIGINT UNSIGNED NOT NULL,
            message_count INT UNSIGNED NOT NULL DEFAULT 0,
            total_message_sent INT UNSIGNED NOT NULL DEFAULT 0,
            member_count INT UNSIGNED NOT NULL DEFAULT 0,
            INDEX idx_channel_threads_guild_archived (guild_id, archived),
            INDEX idx_channel_threads_parent_archived (parent_id, archived, archive_timestamp),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS thread_applied_tags (
            thread_id VARCHAR(20) NOT NULL,
            tag_id VARCHAR(20) NOT NULL,
            position INT UNSIGNED NOT NULL DEFAULT 0,
            PRIMARY KEY (thread_id, tag_id),
            FOREIGN KEY (thread_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (tag_id) REFERENCES forum_tags(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS thread_members (
            thread_id VARCHAR(20) NOT NULL,
            user_id VARCHAR(20) NOT NULL,
            join_timestamp BIGINT UNSIGNED NOT NULL,
            flags INT UNSIGNED NOT NULL DEFAULT 0,
            muted BOOLEAN NOT NULL DEFAULT FALSE,
            mute_config TEXT DEFAULT NULL,
            PRIMARY KEY (thread_id, user_id),
            INDEX idx_thread_members_user (user_id),
            INDEX idx_thread_members_thread_join (thread_id, join_timestamp),
            FOREIGN KEY (thread_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS channel_permission_overwrites (
            type INT UNSIGNED NOT NULL,
            channel_id VARCHAR(20) NOT NULL,
            target_id VARCHAR(20) NOT NULL,
            allow BIGINT UNSIGNED NOT NULL DEFAULT 0,
            deny BIGINT UNSIGNED NOT NULL DEFAULT 0,
            PRIMARY KEY (channel_id, target_id),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS guild_members (
            joined_at BIGINT UNSIGNED NOT NULL,
            guild_id VARCHAR(20) NOT NULL,
            user_id VARCHAR(20) NOT NULL,
            nick VARCHAR(32) DEFAULT NULL,
            PRIMARY KEY (guild_id, user_id),
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS guild_member_roles (
            guild_id VARCHAR(20) NOT NULL,
            user_id VARCHAR(20) NOT NULL,
            role_id VARCHAR(20) NOT NULL,
            PRIMARY KEY (guild_id, user_id, role_id),
            FOREIGN KEY (guild_id, user_id) REFERENCES guild_members(guild_id, user_id) ON DELETE CASCADE,
            FOREIGN KEY (role_id) REFERENCES roles(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS guild_bans (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            banned_at BIGINT UNSIGNED NOT NULL,
            guild_id VARCHAR(20) NOT NULL,
            user_id VARCHAR(20) NOT NULL,
            reason TEXT DEFAULT NULL,
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE,
            UNIQUE (guild_id, user_id)
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            id VARCHAR(20) NOT NULL PRIMARY KEY,
            id_num BIGINT UNSIGNED GENERATED ALWAYS AS (CAST(id AS UNSIGNED)) VIRTUAL,
            timestamp BIGINT UNSIGNED NOT NULL,
            edited_timestamp BIGINT UNSIGNED DEFAULT NULL,
            channel_id VARCHAR(20) NOT NULL,
            author_id VARCHAR(20) NOT NULL,
            type INT UNSIGNED NOT NULL DEFAULT 0,
            flags INT UNSIGNED NOT NULL DEFAULT 0,
            content TEXT DEFAULT NULL,
            INDEX idx_messages_channel_ts (channel_id, timestamp),
            INDEX idx_messages_channel_idnum (channel_id, id_num),
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (author_id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS read_states (
            user_id VARCHAR(20) NOT NULL,
            channel_id VARCHAR(20) NOT NULL,
            last_message_id VARCHAR(20) DEFAULT NULL,
            last_pin_timestamp BIGINT UNSIGNED DEFAULT NULL,
            mention_count INT UNSIGNED NOT NULL DEFAULT 0,
            PRIMARY KEY (user_id, channel_id),
            INDEX idx_read_states_channel (channel_id),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS invites (
            code VARCHAR(16) NOT NULL PRIMARY KEY,
            guild_id VARCHAR(20) NOT NULL,
            channel_id VARCHAR(20) NOT NULL,
            inviter_id VARCHAR(20) NOT NULL,
            uses INT UNSIGNED NOT NULL DEFAULT 0,
            max_uses INT UNSIGNED NOT NULL DEFAULT 0,
            created_at BIGINT UNSIGNED NOT NULL,
            expires_at BIGINT UNSIGNED DEFAULT NULL,
            INDEX idx_invites_guild_created (guild_id, created_at),
            INDEX idx_invites_channel_created (channel_id, created_at),
            FOREIGN KEY (guild_id) REFERENCES guilds(id) ON DELETE CASCADE,
            FOREIGN KEY (channel_id) REFERENCES channels(id) ON DELETE CASCADE,
            FOREIGN KEY (inviter_id) REFERENCES users(id) ON DELETE CASCADE
        )
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
