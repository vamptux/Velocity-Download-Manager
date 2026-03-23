use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct TempTransferLockGuard {
    file: Option<Arc<std::fs::File>>,
    temp_path: String,
    pub warning: Option<String>,
}

impl TempTransferLockGuard {
    pub(super) fn release(&mut self) -> Option<String> {
        let file = self.file.take()?;

        #[cfg(target_os = "windows")]
        {
            fs2::FileExt::unlock(&*file).err().map(|error| {
                format!(
                    "VDM finished writing '{}' but could not release the temp-file transfer lock cleanly before finalize: {error}",
                    Path::new(&self.temp_path).display()
                )
            })
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = file;
            None
        }
    }
}

impl Drop for TempTransferLockGuard {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

pub(super) struct FinalizeDownloadResult {
    pub used_copy_fallback: bool,
    pub warnings: Vec<String>,
}

pub(super) fn acquire_temp_transfer_lock(
    file: Arc<std::fs::File>,
    temp_path: &str,
) -> TempTransferLockGuard {
    #[cfg(target_os = "windows")]
    {
        let path = Path::new(temp_path);
        match fs2::FileExt::try_lock_exclusive(&*file) {
            Ok(()) => TempTransferLockGuard {
                file: Some(file),
                temp_path: temp_path.to_string(),
                warning: None,
            },
            Err(error) => TempTransferLockGuard {
                file: None,
                temp_path: temp_path.to_string(),
                warning: Some(format!(
                    "VDM could not keep an exclusive write lock on the temp file '{}' during transfer: {error}. Real-time antivirus or another process may still slow disk writes on this volume.",
                    path.display()
                )),
            },
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = file;
        let _ = temp_path;
        TempTransferLockGuard {
            file: None,
            temp_path: temp_path.to_string(),
            warning: None,
        }
    }
}

pub(super) fn reset_temp_file_path(temp_path: &str) -> Result<(), String> {
    let path = Path::new(temp_path);
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("Failed resetting temp file '{}': {error}", path.display()))?;
    }
    Ok(())
}

pub(super) fn query_available_space(path: &Path) -> Option<u64> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return fs2::available_space(candidate).ok();
        }
        current = candidate.parent();
    }
    None
}

pub(super) fn finalize_download_file(
    temp_path: &str,
    target_path: &str,
) -> Result<FinalizeDownloadResult, String> {
    let temp = Path::new(temp_path);
    let target = Path::new(target_path);

    let mut rename_file = |from: &Path, to: &Path| fs::rename(from, to);
    finalize_download_paths(temp, target, &mut rename_file)
}

fn finalize_download_paths<R>(
    temp: &Path,
    target: &Path,
    rename_file: &mut R,
) -> Result<FinalizeDownloadResult, String>
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
{
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Failed creating target directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    if target.exists() && !target.is_file() {
        return Err(format!(
            "Target path '{}' already exists and is not a file.",
            target.display()
        ));
    }

    let replaced_target = park_existing_target_with_rename(target, rename_file)?;

    match rename_file(temp, target) {
        Ok(()) => {
            let warnings = cleanup_replaced_target(replaced_target.as_deref());
            Ok(FinalizeDownloadResult {
                used_copy_fallback: false,
                warnings,
            })
        }
        Err(error) if is_cross_volume_rename_error(&error) => {
            if let Err(copy_error) = copy_file_with_flush(temp, target) {
                let _ = fs::remove_file(target);
                let restore_error = restore_replaced_target_with_rename(
                    replaced_target.as_deref(),
                    target,
                    rename_file,
                )
                .err()
                .map(|message| format!(" Previous target restore also failed: {message}"))
                .unwrap_or_default();
                return Err(format!("{copy_error}{restore_error}"));
            }

            let mut warnings = cleanup_replaced_target(replaced_target.as_deref());
            if let Err(remove_error) = fs::remove_file(temp) {
                warnings.push(format!(
                    "Completed file was copied into place across volumes, but VDM could not remove the temp file automatically: {remove_error}"
                ));
            }

            Ok(FinalizeDownloadResult {
                used_copy_fallback: true,
                warnings,
            })
        }
        Err(error) => {
            let restore_error = restore_replaced_target_with_rename(
                replaced_target.as_deref(),
                target,
                rename_file,
            )
            .err()
            .map(|message| format!(" Previous target restore also failed: {message}"))
            .unwrap_or_default();
            Err(format!(
                "Failed moving temp file into target location: {error}.{restore_error}"
            ))
        }
    }
}

fn park_existing_target_with_rename<R>(
    target: &Path,
    rename_file: &mut R,
) -> Result<Option<PathBuf>, String>
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
{
    if !target.exists() {
        return Ok(None);
    }

    let file_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("download");

    for attempt in 0..32_u32 {
        let backup = target.with_file_name(format!("{file_name}.vdm-replaced-{attempt}"));
        if backup.exists() {
            continue;
        }
        rename_file(target, &backup).map_err(|error| {
            format!(
                "Failed preparing existing target '{}' for replacement: {error}",
                target.display()
            )
        })?;
        return Ok(Some(backup));
    }

    Err(format!(
        "Failed preparing existing target '{}' for replacement because VDM could not reserve a backup name.",
        target.display()
    ))
}

fn cleanup_replaced_target(backup: Option<&Path>) -> Vec<String> {
    let Some(backup) = backup else {
        return Vec::new();
    };

    if let Err(error) = fs::remove_file(backup) {
        return vec![format!(
            "VDM finalized the completed file but could not delete the previous target backup '{}': {error}",
            backup.display()
        )];
    }

    Vec::new()
}

fn restore_replaced_target_with_rename<R>(
    backup: Option<&Path>,
    target: &Path,
    rename_file: &mut R,
) -> Result<(), String>
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
{
    let Some(backup) = backup else {
        return Ok(());
    };

    if target.exists() {
        fs::remove_file(target).map_err(|error| {
            format!(
                "Failed removing partial target '{}' before restore: {error}",
                target.display()
            )
        })?;
    }

    rename_file(backup, target).map_err(|error| {
        format!(
            "Failed restoring the previous target '{}' from '{}': {error}",
            target.display(),
            backup.display()
        )
    })
}

fn copy_file_with_flush(temp: &Path, target: &Path) -> Result<(), String> {
    fs::copy(temp, target).map_err(|error| {
        format!(
            "Failed copying completed temp file into target location '{}': {error}",
            target.display()
        )
    })?;

    OpenOptions::new()
        .read(true)
        .write(true)
        .open(target)
        .and_then(|file| file.sync_all())
        .map_err(|error| {
            format!(
                "Failed flushing copied target file '{}': {error}",
                target.display()
            )
        })
}

fn is_cross_volume_rename_error(error: &io::Error) -> bool {
    #[cfg(target_os = "windows")]
    const CROSS_VOLUME_OS_ERROR: i32 = 17;
    #[cfg(not(target_os = "windows"))]
    const CROSS_VOLUME_OS_ERROR: i32 = 18;

    error.raw_os_error() == Some(CROSS_VOLUME_OS_ERROR)
}

pub(super) fn open_in_file_manager(path: &Path, is_file: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        if is_file {
            let resolved_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let path_lossy = resolved_path.to_string_lossy();
            let clean_path = path_lossy.strip_prefix("\\\\?\\").unwrap_or(&path_lossy);
            let clean_path = clean_path.replace('/', "\\");
            let selection_arg = format!("/select,{clean_path}");
            // `explorer /select,` delegates to the already-running Explorer process and
            // exits with a non-zero code even on success.  Using spawn() (fire-and-forget)
            // avoids treating that non-zero code as failure and accidentally opening a
            // second window via the fallback path.
            Command::new("explorer")
                .arg(&selection_arg)
                .spawn()
                .map_err(|e| {
                    format!(
                        "Failed to open Explorer for '{}': {e}",
                        resolved_path.display()
                    )
                })?;
            return Ok(());
        }
        let resolved_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_lossy = resolved_path.to_string_lossy();
        let clean_path = path_lossy.strip_prefix("\\\\?\\").unwrap_or(&path_lossy);
        let clean_path = clean_path.replace('/', "\\");
        Command::new("explorer")
            .arg(&*clean_path)
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to open Explorer for '{}': {e}",
                    resolved_path.display()
                )
            })?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let mut command = Command::new("open");
        if is_file {
            command.arg("-R");
        }
        command.arg(path.as_os_str());
        run_file_manager_command(&mut command, path)?;
        Ok(())
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let target = if is_file {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        let mut command = Command::new("xdg-open");
        command.arg(target.as_os_str());
        run_file_manager_command(&mut command, target)?;
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
fn run_file_manager_command(command: &mut Command, path: &Path) -> Result<(), String> {
    let status = command.status().map_err(|error| {
        format!(
            "Failed opening '{}' in file manager: {error}",
            path.display()
        )
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "File manager command failed for '{}' with status: {status}.",
            path.display()
        ))
    }
}

/// Open a file with its default associated application (equivalent to double-clicking in Explorer).
pub(super) fn open_file_with_default_app(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let path_lossy = resolved.to_string_lossy();
        let clean_path = path_lossy.strip_prefix("\\\\?\\").unwrap_or(&path_lossy).replace('/', "\\");
        Command::new("cmd")
            .args(["/c", "start", "", &*clean_path])
            .spawn()
            .map_err(|e| format!("Failed to open file '{}': {e}", resolved.display()))?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path.as_os_str())
            .spawn()
            .map_err(|e| format!("Failed to open file '{}': {e}", path.display()))?;
        Ok(())
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path.as_os_str())
            .spawn()
            .map_err(|e| format!("Failed to open file '{}': {e}", path.display()))?;
        Ok(())
    }
}
