use chrono::{DateTime, Utc};

use crate::domain::session::State;

use super::{Owner, Ownership, OwnershipError, Path, ports};

pub async fn claim(
    store: &impl ports::Store,
    owner: &Owner,
    path: &Path,
    now: DateTime<Utc>,
) -> Result<Ownership, OwnershipError> {
    let owned = store
        .update_ownership(owner, path, |session, launch, existing| {
            let session =
                session.ok_or_else(|| OwnershipError::NotRegistered(owner.session.clone()))?;
            if session.launch() != owner.launch.as_ref()
                || session.state() != &State::Active
                || !match (&owner.launch, launch) {
                    (None, None) => true,
                    (Some(id), Some(launch)) => {
                        launch.id == *id
                            && launch.host == *owner.session.host()
                            && launch.session.as_ref() == Some(&owner.session)
                            && launch.state == State::Active
                    }
                    _ => false,
                }
            {
                return Err(OwnershipError::Binding);
            }
            match existing {
                Some(existing) if existing.owner == *owner => Ok(Some(existing)),
                Some(existing) => Err(OwnershipError::Claimed(existing)),
                None => Ok(Some(Ownership {
                    path: path.clone(),
                    owner: owner.clone(),
                    claimed_at: now,
                })),
            }
        })
        .await??;
    owned.ok_or(OwnershipError::Binding)
}

pub async fn release(
    store: &impl ports::Store,
    owner: &Owner,
    path: &Path,
) -> Result<(), OwnershipError> {
    store
        .update_ownership(owner, path, |_, _, existing| match existing {
            Some(existing) if existing.owner != *owner => Err(OwnershipError::Claimed(existing)),
            _ => Ok(None),
        })
        .await??;
    Ok(())
}
