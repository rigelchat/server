mod crud;
mod gateway;
mod template;

pub use crud::{create_guild, delete_guild, get_guild, get_user_guilds, update_guild};
pub use gateway::{build_full_gateway_guild, build_gateway_guilds_batch};
