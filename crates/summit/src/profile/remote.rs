use http::Uri;
use service::{Remote, database::Transaction};

use crate::profile;

pub async fn create(
    tx: &mut Transaction,
    profile: profile::Id,
    index_uri: Uri,
    name: String,
    priority: u64,
) -> Result<Remote, sqlx::Error> {
    sqlx::query(
        "
        INSERT INTO profile_remote
        (
          profile_id,
          index_uri,
          name,
          priority
        )
        VALUES (?,?,?,?);
        ",
    )
    .bind(i64::from(profile))
    .bind(index_uri.to_string())
    .bind(&name)
    .bind(priority as i64)
    .execute(tx.as_mut())
    .await?;

    Ok(Remote {
        index_uri,
        name,
        priority,
    })
}
