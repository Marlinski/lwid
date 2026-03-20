//! Background reaper for expired projects.
//!
//! The reaper runs as a periodic tokio task that scans all projects, finds
//! those past their `expires_at` deadline, and deletes them along with any
//! blobs that are not shared with other live projects.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tracing::{error, info, warn};

use lwid_common::cid::Cid;
use lwid_common::kv::KvStore;
use lwid_common::project::ProjectStore;
use lwid_common::store::BlobStore;

/// Default interval between reaper sweeps.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60); // 5 minutes

/// Spawn the reaper as a background task.
///
/// The reaper runs forever, sleeping `SWEEP_INTERVAL` between sweeps. It is
/// designed to tolerate errors gracefully — a failed sweep is logged and
/// retried on the next cycle.
pub fn spawn(projects: Arc<dyn ProjectStore>, blobs: Arc<dyn BlobStore>, kv: Arc<dyn KvStore>) {
    tokio::spawn(async move {
        info!("reaper started (sweep interval: {}s)", SWEEP_INTERVAL.as_secs());
        loop {
            tokio::time::sleep(SWEEP_INTERVAL).await;
            sweep(&*projects, &*blobs, &*kv);
        }
    });
}

/// Perform a single reaper sweep.
///
/// Lists all projects, identifies expired ones, collects the set of blob CIDs
/// owned by *live* projects, then deletes each expired project and any of its
/// blobs that are not referenced by a live project.
fn sweep(projects: &dyn ProjectStore, blobs: &dyn BlobStore, kv: &dyn KvStore) {
    let ids = match projects.list() {
        Ok(ids) => ids,
        Err(e) => {
            error!("reaper: failed to list projects: {e}");
            return;
        }
    };

    if ids.is_empty() {
        return;
    }

    let now = Utc::now();

    // First pass: partition into expired and live, collecting live blob CIDs.
    let mut expired_ids = Vec::new();
    let mut live_blob_cids: BTreeSet<String> = BTreeSet::new();

    for id in &ids {
        match projects.get(id) {
            Ok(project) => {
                if project.is_expired(now) {
                    expired_ids.push(id.clone());
                } else {
                    live_blob_cids.extend(project.blob_cids.iter().cloned());
                }
            }
            Err(e) => {
                warn!("reaper: failed to read project {id}: {e}");
            }
        }
    }

    if expired_ids.is_empty() {
        return;
    }

    info!("reaper: found {} expired project(s)", expired_ids.len());

    // Second pass: delete each expired project and its exclusive blobs.
    for id in &expired_ids {
        match projects.delete(id) {
            Ok(project) => {
                let mut deleted_blobs = 0u64;
                let mut kept_blobs = 0u64;

                for cid_str in &project.blob_cids {
                    if live_blob_cids.contains(cid_str) {
                        kept_blobs += 1;
                        continue;
                    }

                    if let Ok(cid) = Cid::from_string(cid_str) {
                        if let Err(e) = blobs.delete(&cid) {
                            warn!("reaper: failed to delete blob {cid_str}: {e}");
                        } else {
                            deleted_blobs += 1;
                        }
                    }
                }

                // Clean up KV store data for this project.
                match kv.delete_all(id) {
                    Ok(()) => {
                        info!(project_id = %id, "reaper: cleaned up store data");
                    }
                    Err(e) => {
                        warn!("reaper: failed to clean up store data for {id}: {e}");
                    }
                }

                info!(
                    project_id = %id,
                    deleted_blobs,
                    kept_blobs,
                    "reaper: deleted expired project",
                );
            }
            Err(e) => {
                error!("reaper: failed to delete project {id}: {e}");
            }
        }
    }
}
