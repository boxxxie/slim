use std::{
    env, fs,
    os::unix::{fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

use compact_str::CompactString;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::{
    domain::{IssueDetailPage, IssueSort, ListViewKind},
    id::{IssueId, ProjectScope},
    result::{Result, SlimError},
};

pub const HOT_RELOAD_ENV_VAR: &str = "SLIM_HOT_RELOAD_FILE";

#[derive(Debug, Clone, PartialEq, Eq)]
struct BinarySignature {
    inode: u64,
    mtime_sec: i64,
    mtime_nsec: i64,
    size: u64,
}

#[derive(Debug, Clone)]
pub struct BinaryWatcher {
    primary_path: PathBuf,
    initial_primary: Option<BinarySignature>,
    cargo_release_path: PathBuf,
    initial_cargo: Option<BinarySignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotReloadState {
    pub active_list_view: ListViewKind,
    pub project_scope: ProjectScope,
    pub sort: IssueSort,
    pub search_query: CompactString,
    pub selected_row: Option<usize>,
    pub open_detail_issue_id: Option<IssueId>,
    pub open_detail_page: Option<IssueDetailPage>,
}

/// Resolves the on-disk binary path even after `cargo build` or `cp` unlinks and replaces the running executable.
#[must_use]
pub fn current_binary_path() -> PathBuf {
    let Ok(exe) = env::current_exe() else {
        return PathBuf::from("slim");
    };
    let lossy = exe.to_string_lossy();
    if let Some(stripped) = lossy.strip_suffix(" (deleted)") {
        PathBuf::from(stripped)
    } else {
        exe
    }
}

fn read_signature(path: &Path) -> Option<(BinarySignature, Option<SystemTime>)> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 {
        return None;
    }
    let sig = BinarySignature {
        inode: meta.ino(),
        mtime_sec: meta.mtime(),
        mtime_nsec: meta.mtime_nsec(),
        size: meta.len(),
    };
    Some((sig, meta.modified().ok()))
}

fn is_settled_update(path: &Path, initial: &Option<BinarySignature>) -> bool {
    let Some(ref init_sig) = initial else {
        return false;
    };
    let Some((current, modified)) = read_signature(path) else {
        return false;
    };
    if &current == init_sig {
        return false;
    }
    if let Some(mtime) = modified {
        if mtime
            .elapsed()
            .is_ok_and(|age| age < Duration::from_millis(300))
        {
            return false;
        }
    }
    fs::File::open(path).is_ok()
}

impl Default for BinaryWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryWatcher {
    #[must_use]
    pub fn new() -> Self {
        let primary_path = current_binary_path();
        let initial_primary = read_signature(&primary_path).map(|(sig, _)| sig);
        let cargo_release_path = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/target/release/slim"
        ));
        let initial_cargo = read_signature(&cargo_release_path).map(|(sig, _)| sig);
        Self {
            primary_path,
            initial_primary,
            cargo_release_path,
            initial_cargo,
        }
    }

    /// Returns `Some(updated_path)` when the on-disk executable (or `target/release/slim`) has been
    /// replaced by a newly compiled binary and settled for at least 300ms.
    #[must_use]
    pub fn updated_binary_path(&self) -> Option<PathBuf> {
        if is_settled_update(&self.primary_path, &self.initial_primary) {
            return Some(self.primary_path.clone());
        }
        if self.cargo_release_path != self.primary_path
            && is_settled_update(&self.cargo_release_path, &self.initial_cargo)
        {
            return Some(self.cargo_release_path.clone());
        }
        None
    }

    #[must_use]
    pub fn is_binary_updated(&self) -> bool {
        self.updated_binary_path().is_some()
    }
}

impl HotReloadState {
    pub fn save_to_disk(&self) -> Result<PathBuf> {
        let pid = std::process::id();
        let path = env::temp_dir().join(format!("slim-hot-reload-{pid}.json"));
        let payload = serde_json::to_string(self)
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        fs::write(&path, payload)
            .map_err(|e| SlimError::GeneralError(e.to_string().into()))?;
        Ok(path)
    }

    #[must_use]
    pub fn load_from_env() -> Option<Self> {
        let path_str = env::var(HOT_RELOAD_ENV_VAR).ok()?;
        let path = PathBuf::from(path_str);
        let raw = fs::read_to_string(&path).ok()?;
        drop(fs::remove_file(&path));
        match serde_json::from_str::<Self>(&raw) {
            Ok(state) => {
                info!(
                    view = ?state.active_list_view,
                    "Restored slim hot-reload state"
                );
                Some(state)
            }
            Err(err) => {
                warn!("Failed to deserialize slim hot-reload state: {err}");
                None
            }
        }
    }
}

/// Replaces the current process image in-place using `execv` (`CommandExt::exec`).
pub fn exec_new_binary(updated_path: &Path, state_file: &Path) -> Result<()> {
    let exe = current_binary_path();
    if updated_path != exe {
        let tmp_dest = exe.with_extension("hot_reload_tmp");
        if fs::copy(updated_path, &tmp_dest).is_ok() {
            let _ = fs::rename(&tmp_dest, &exe);
        }
    }
    let target_exe = if exe.exists() { exe } else { updated_path.to_path_buf() };
    let args: Vec<String> = env::args().skip(1).collect();
    info!(
        "Hot-reloading into {} with state file {}",
        target_exe.display(),
        state_file.display()
    );

    let err = Command::new(&target_exe)
        .args(&args)
        .env(HOT_RELOAD_ENV_VAR, state_file)
        .exec();

    Err(SlimError::GeneralError(err.to_string().into()))
}
