//! Saving a turned or mirrored image into its own file, with glycin's
//! editor (JPEG and PNG). A JPEG with an orientation tag only gets that tag
//! changed — the pixels stay as they are; otherwise the image is written
//! again, losslessly for PNG.

use std::{collections::HashSet, path::PathBuf};

use glycin::{Editor, EditOutcome, Operation, Operations};
use gufo_common::orientation::Rotation;

/// The MIME types glycin can edit.
pub fn editable_types() -> HashSet<String> {
    async_io::block_on(Editor::supported_formats())
        .into_keys()
        .map(|m| m.as_str().to_owned())
        .collect()
}

/// Applies `turns` quarter turns clockwise, after mirroring left to right
/// if `mirrored`, to the file at `path`. The result arrives on the channel.
pub fn save(path: PathBuf, turns: u8, mirrored: bool) -> async_channel::Receiver<Result<(), String>> {
    let (tx, rx) = async_channel::bounded(1);
    let spawned = std::thread::Builder::new()
        .name("orbit-view-save".into())
        .spawn(move || {
            let result = async_io::block_on(run(&path, turns, mirrored));
            let _ = tx.send_blocking(result);
        });
    if let Err(err) = spawned {
        log::error!("starting the save: {err}");
    }
    rx
}

fn operations(turns: u8, mirrored: bool) -> Operations {
    let mut list = Vec::new();
    if mirrored {
        list.push(Operation::MirrorHorizontally);
    }
    // glycin turns counter-clockwise.
    match (4 - turns % 4) % 4 {
        1 => list.push(Operation::Rotate(Rotation::_90)),
        2 => list.push(Operation::Rotate(Rotation::_180)),
        3 => list.push(Operation::Rotate(Rotation::_270)),
        _ => {}
    }
    Operations::new(list)
}

async fn run(path: &std::path::Path, turns: u8, mirrored: bool) -> Result<(), String> {
    let file = gio20::File::for_path(path);
    let sparse = Editor::new(file.clone())
        .apply_sparse(operations(turns, mirrored))
        .await
        .map_err(|e| e.to_string())?;
    if let EditOutcome::Changed = sparse.apply_to(file.clone()).await.map_err(|e| e.to_string())? {
        return Ok(());
    }
    let data = Editor::new(file)
        .apply_complete(&operations(turns, mirrored))
        .await
        .map_err(|e| e.to_string())?
        .get_full()
        .map_err(|e| e.to_string())?;
    // Into a file beside it, then over it, keeping its permissions.
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = path.with_file_name(format!(".{name}.orbit-view"));
    std::fs::write(&temporary, &data).map_err(|e| e.to_string())?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&temporary, meta.permissions());
    }
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clockwise_turns_become_counter_clockwise() {
        let ops = operations(1, false);
        assert_eq!(ops.operations(), &[Operation::Rotate(Rotation::_270)]);
        let ops = operations(3, true);
        assert_eq!(
            ops.operations(),
            &[Operation::MirrorHorizontally, Operation::Rotate(Rotation::_90)]
        );
        assert!(operations(0, false).operations().is_empty());
    }
}
