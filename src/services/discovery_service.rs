use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::guild::{
    DEFAULT_PRIMARY_LANGUAGE, DiscoveryCategory, FEATURE_DISCOVERABLE, VerificationLevel,
};
use crate::constants::limits::{
    DEFAULT_DISCOVERY_LIMIT, MAX_DISCOVERY_LIMIT, MAX_SEARCH_QUERY_LENGTH,
};
use crate::error::AppError;
use crate::models::rest::discoverable_guilds::{
    DiscoverableGuild as RestDiscoverableGuild, DiscoverableGuildsResponse,
};
use crate::models::rest::discovery::{
    CategoryCount, DiscoverableGuild, DiscoverySearchQuery, GuildDiscoveryResponse,
};

#[derive(FromRow)]
struct DiscoverableGuildRow {
    id: String,
    name: String,
    icon: Option<String>,
    banner: Option<String>,
    description: Option<String>,
    member_count: i64,
}

async fn fetch_discoverable_rows(
    state: &AppState,
    search: Option<&str>,
) -> Result<Vec<DiscoverableGuildRow>, AppError> {
    let mut qb = QueryBuilder::<MySql>::new(
        r#"
        SELECT
            g.id, g.name, g.icon, g.banner, g.description,
            COUNT(DISTINCT gm.user_id) AS member_count
        FROM guilds g
        LEFT JOIN guild_members gm ON g.id = gm.guild_id
        WHERE g.discoverable = TRUE
        "#,
    );

    if let Some(q) = search
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(MAX_SEARCH_QUERY_LENGTH).collect::<String>())
    {
        let escaped = q
            .to_ascii_lowercase()
            .replace('!', "!!")
            .replace('%', "!%")
            .replace('_', "!_");
        let pattern = format!("%{escaped}%");
        qb.push(" AND (LOWER(g.name) LIKE ");
        qb.push_bind(pattern.clone());
        qb.push(" ESCAPE '!' OR LOWER(COALESCE(g.description, '')) LIKE ");
        qb.push_bind(pattern);
        qb.push(" ESCAPE '!')");
    }

    qb.push(" GROUP BY g.id ORDER BY member_count DESC, g.name ASC");

    Ok(qb
        .build_query_as::<DiscoverableGuildRow>()
        .fetch_all(&state.db)
        .await?)
}

pub async fn get_discovery_guilds(
    state: &AppState,
    params: DiscoverySearchQuery,
) -> Result<GuildDiscoveryResponse, AppError> {
    if let Some(cat) = params.category
        && cat != DiscoveryCategory::Other as u8
    {
        return Ok(GuildDiscoveryResponse {
            guilds: vec![],
            total: 0,
            category_counts: vec![],
        });
    }

    let rows = fetch_discoverable_rows(state, params.query.as_deref()).await?;

    let total = rows.len() as u64;
    let offset = params.offset.unwrap_or(0) as usize;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_DISCOVERY_LIMIT)
        .clamp(1, MAX_DISCOVERY_LIMIT) as usize;

    let page: Vec<DiscoverableGuildRow> = rows.into_iter().skip(offset).take(limit).collect();
    let page_ids: Vec<String> = page.iter().map(|r| r.id.clone()).collect();
    let online_counts = state.hub.count_guilds_online(&page_ids);

    let guilds: Vec<DiscoverableGuild> = page
        .into_iter()
        .map(|row| {
            let online_count = online_counts.get(&row.id).copied().unwrap_or(0);
            DiscoverableGuild {
                id: row.id,
                name: row.name,
                icon: row.icon,
                banner: row.banner,
                description: row.description,
                category_type: DiscoveryCategory::Other,
                primary_language: DEFAULT_PRIMARY_LANGUAGE.to_string(),
                custom_tags: vec![],
                member_count: row.member_count.max(0) as u64,
                online_count,
                features: vec![FEATURE_DISCOVERABLE.to_string()],
                badge_ids: vec![],
                verification_level: VerificationLevel::Low,
            }
        })
        .collect();

    let category_counts = if total > 0 {
        vec![CategoryCount {
            category_type: DiscoveryCategory::Other,
            count: total,
        }]
    } else {
        vec![]
    };

    Ok(GuildDiscoveryResponse {
        total,
        guilds,
        category_counts,
    })
}

pub async fn get_discoverable_guilds(
    state: &AppState,
) -> Result<DiscoverableGuildsResponse, AppError> {
    let rows = fetch_discoverable_rows(state, None).await?;
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let online_counts = state.hub.count_guilds_online(&ids);

    let guilds: Vec<RestDiscoverableGuild> = rows
        .into_iter()
        .map(|row| {
            let presence_count = online_counts.get(&row.id).copied().unwrap_or(0);
            RestDiscoverableGuild {
                id: row.id,
                name: row.name,
                description: row.description,
                icon: row.icon,
                banner: row.banner,
                splash: None,
                discovery_splash: None,
                features: vec![FEATURE_DISCOVERABLE.to_string()],
                approximate_presence_count: presence_count,
                approximate_member_count: row.member_count.max(0) as u64,
                premium_subscription_count: 0,
            }
        })
        .collect();

    let total = guilds.len() as u64;

    Ok(DiscoverableGuildsResponse {
        limit: total,
        offset: 0,
        total,
        guilds,
    })
}

pub async fn join_guild(state: &AppState, user_id: &str, guild_id: &str) -> Result<(), AppError> {
    let discoverable: Option<bool> =
        sqlx::query_scalar("SELECT discoverable FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_optional(&state.db)
            .await?;

    if discoverable != Some(true) {
        return Err(AppError::unknown_guild());
    }

    crate::services::member_service::add_member_to_guild(state, user_id, guild_id).await
}
