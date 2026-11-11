use sea_orm::{ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use crate::domain::session::ports::StoreError;
use crate::domain::session::{Launch, LaunchId, Session};
use crate::domain::worktree::{Owner, Ownership, Path, ports};

use super::entity::{session, worktree_ownership};
use super::{Store, corrupt, find_launch_row, find_row, handle_of, load, to_launch, unavailable};

impl ports::Store for Store {
    async fn update_ownership<E: Send>(
        &self,
        owner: &Owner,
        path: &Path,
        change: impl FnOnce(
            Option<Session>,
            Option<Launch>,
            Option<Ownership>,
        ) -> Result<Option<Ownership>, E>
        + Send,
    ) -> Result<Result<Option<Ownership>, E>, StoreError> {
        let txn = self.write().await?;
        let session = find_row(&txn, &owner.session).await?;
        let session_id = session.as_ref().map(|row| row.id);
        let session = load(&txn, session.into_iter().collect()).await?.pop();
        let launch = match &owner.launch {
            Some(id) => match find_launch_row(&txn, id).await? {
                Some(row) => Some(to_launch(&txn, row).await?),
                None => None,
            },
            None => None,
        };
        let key = path.as_str();
        let existing = worktree_ownership::Entity::find_by_id(key)
            .one(&txn)
            .await
            .map_err(unavailable)?;
        let existing = match existing {
            Some(row) => Some(ownership(&txn, row).await?),
            None => None,
        };
        let updated = match change(session, launch, existing) {
            Ok(updated) => updated,
            Err(error) => {
                txn.rollback().await.map_err(unavailable)?;
                return Ok(Err(error));
            }
        };
        match &updated {
            Some(owned) => {
                let session_id = session_id
                    .ok_or_else(|| corrupt("ownership has no registered session".into()))?;
                worktree_ownership::Entity::insert(worktree_ownership::ActiveModel {
                    path: Set(key.to_owned()),
                    session_id: Set(session_id),
                    launch_id: Set(owned.owner.launch.as_ref().map(|id| id.as_str().to_owned())),
                    claimed_at: Set(owned.claimed_at),
                })
                .on_conflict(
                    sea_orm::sea_query::OnConflict::column(worktree_ownership::Column::Path)
                        .update_column(worktree_ownership::Column::Path)
                        .to_owned(),
                )
                .exec_without_returning(&txn)
                .await
                .map_err(unavailable)?;
            }
            None => {
                worktree_ownership::Entity::delete_by_id(key)
                    .exec(&txn)
                    .await
                    .map_err(unavailable)?;
            }
        }
        txn.commit().await.map_err(unavailable)?;
        Ok(Ok(updated))
    }

    async fn worktrees(&self, owner: &Owner) -> Result<Vec<Ownership>, StoreError> {
        let Some(session) = find_row(&self.db, &owner.session).await? else {
            return Ok(Vec::new());
        };
        let query = worktree_ownership::Entity::find()
            .filter(worktree_ownership::Column::SessionId.eq(session.id));
        let query = match &owner.launch {
            Some(id) => query.filter(worktree_ownership::Column::LaunchId.eq(id.as_str())),
            None => query.filter(worktree_ownership::Column::LaunchId.is_null()),
        };
        let mut owned = Vec::new();
        for row in query
            .order_by_asc(worktree_ownership::Column::Path)
            .all(&self.db)
            .await
            .map_err(unavailable)?
        {
            owned.push(ownership(&self.db, row).await?);
        }
        Ok(owned)
    }
}

async fn ownership(
    db: &impl sea_orm::ConnectionTrait,
    row: worktree_ownership::Model,
) -> Result<Ownership, StoreError> {
    let session = session::Entity::find_by_id(row.session_id)
        .one(db)
        .await
        .map_err(unavailable)?
        .ok_or_else(|| corrupt("worktree owner session is missing".into()))?;
    Ok(Ownership {
        path: Path::new(std::path::Path::new(&row.path))
            .map_err(|error| corrupt(error.to_string()))?,
        owner: Owner {
            session: handle_of(&session)?,
            launch: row
                .launch_id
                .as_deref()
                .map(LaunchId::parse)
                .transpose()
                .map_err(|error| corrupt(error.to_string()))?,
        },
        claimed_at: row.claimed_at,
    })
}
