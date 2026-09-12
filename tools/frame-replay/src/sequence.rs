use crate::frame::{self, Frame};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Entry {
    pub frame: PathBuf,
    pub reference: PathBuf,
}

pub fn load(path: &Path) -> Result<Vec<Entry>> {
    let manifest: Value = serde_json::from_slice(&fs::read(path)?)?;
    let directory = path.parent().context("manifest has no parent directory")?;
    parse(&manifest, directory)
}

fn parse(manifest: &Value, directory: &Path) -> Result<Vec<Entry>> {
    ensure!(
        manifest["format"] == "KFXSEQ01",
        "unsupported sequence format"
    );
    let frames = manifest["frames"]
        .as_array()
        .context("sequence frames must be an array")?;
    ensure!(
        !frames.is_empty(),
        "sequence must contain at least one frame"
    );
    frames
        .iter()
        .map(|entry| {
            let resolve = |key: &str| -> Result<PathBuf> {
                let name = entry[key]
                    .as_str()
                    .with_context(|| format!("sequence entry requires {key} path"))?;
                ensure!(!name.is_empty(), "sequence path cannot be empty");
                Ok(directory.join(name))
            };
            Ok(Entry {
                frame: resolve("frame")?,
                reference: resolve("reference")?,
            })
        })
        .collect()
}

fn fixture_frames() -> Vec<(&'static str, Frame)> {
    let baseline = Frame::fixture();
    let mut palette = baseline.clone();
    palette.palette[17 * 4..17 * 4 + 3].copy_from_slice(&[240, 9, 31]);
    let mut alpha = palette.clone();
    alpha.palette[129 * 4 + 3] = 0;
    let mut indices = alpha.clone();
    indices
        .indices
        .iter_mut()
        .for_each(|i| *i = i.wrapping_add(1));
    let mut next_indices = indices.clone();
    next_indices.indices.reverse();
    let mut resized = next_indices.clone();
    resized.width = 63;
    resized.height = 23;
    resized
        .indices
        .truncate((resized.width * resized.height) as usize);
    vec![
        ("baseline", baseline.clone()),
        ("palette-rgb-only", palette),
        ("palette-alpha-only", alpha),
        ("indices-first", indices),
        ("indices-consecutive", next_indices),
        ("dimensions-change", resized),
        ("baseline-restored", baseline),
    ]
}

pub fn fixture(directory: &Path) -> Result<()> {
    ensure!(!directory.exists(), "fixture directory already exists");
    fs::create_dir_all(directory)?;
    let mut entries = Vec::new();
    for (index, (name, frame)) in fixture_frames().into_iter().enumerate() {
        let frame_name = format!("frame-{index:04}.kfx");
        let reference_name = format!("reference-{index:04}.png");
        frame.save(&directory.join(&frame_name))?;
        frame::write_png(
            &directory.join(&reference_name),
            frame.width,
            frame.height,
            &frame.rgba(),
        )?;
        entries.push(json!({"name":name, "frame":frame_name, "reference":reference_name}));
    }
    fs::write(
        directory.join("sequence.json"),
        serde_json::to_vec_pretty(&json!({"format":"KFXSEQ01", "frames":entries}))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::compare;

    #[test]
    fn resolves_manifest_paths_in_order() {
        let entries = parse(
            &json!({"format":"KFXSEQ01", "frames":[
                {"frame":"second.kfx", "reference":"second.png"},
                {"frame":"first.kfx", "reference":"first.png"}
            ]}),
            Path::new("capture"),
        )
        .unwrap();
        assert_eq!(entries[0].frame, Path::new("capture/second.kfx"));
        assert_eq!(entries[1].reference, Path::new("capture/first.png"));
    }

    #[test]
    fn rejects_invalid_manifests() {
        for manifest in [
            json!({"format":"KFXSEQ02", "frames":[{}]}),
            json!({"format":"KFXSEQ01", "frames":[]}),
            json!({"format":"KFXSEQ01", "frames":{}}),
            json!({"format":"KFXSEQ01", "frames":[{"frame":"a.kfx"}]}),
            json!({"format":"KFXSEQ01", "frames":[{"frame":"", "reference":"a.png"}]}),
        ] {
            assert!(parse(&manifest, Path::new(".")).is_err());
        }
    }

    #[test]
    fn sequence_exercises_independent_changes_and_restoration() {
        let frames = fixture_frames();
        let frames: Vec<_> = frames.iter().map(|(_, frame)| frame).collect();
        assert_eq!(frames[0].indices, frames[1].indices);
        assert_eq!(frames[1].indices, frames[2].indices);
        assert_ne!(frames[0].rgba(), frames[1].rgba());
        assert_ne!(frames[1].rgba(), frames[2].rgba());
        for index in [3, 4] {
            assert_eq!(frames[index - 1].palette, frames[index].palette);
            assert_ne!(frames[index - 1].indices, frames[index].indices);
            assert_ne!(frames[index - 1].rgba(), frames[index].rgba());
        }
        assert_ne!(
            (frames[4].width, frames[4].height),
            (frames[5].width, frames[5].height)
        );
        assert_eq!(frames[0].rgba(), frames[6].rgba());
        assert_eq!(&frames[0].palette[128 * 4..129 * 4], &[128, 127, 128, 0]);
    }

    #[test]
    fn detects_alpha_and_hidden_rgb_palette_corruption() {
        let reference = Frame::fixture();
        let expected = reference.rgba();
        for offset in [128 * 4, 128 * 4 + 3, 129 * 4 + 3] {
            let mut corrupted = reference.clone();
            corrupted.palette[offset] ^= 1;
            let count = reference
                .indices
                .iter()
                .filter(|&&i| usize::from(i) == offset / 4)
                .count();
            let (changed, max, _) = compare(&expected, &corrupted.rgba()).unwrap();
            assert!(count > 0);
            assert_eq!((changed, max), (count, 1));
        }
    }
}
