mod offline;
mod report;
mod sequence;

use anyhow::{Context, Result, bail, ensure};
use keeperfx_frame_replay::frame::{self, Frame};
use offline::Replay;
use serde_json::json;
use std::time::Instant;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = args.next().context("usage: keeperfx-frame-replay FRAME.kfx --out DIRECTORY [--reference PNG] [--scale 1..8]\n       keeperfx-frame-replay --fixture DIRECTORY\n       keeperfx-frame-replay --sequence MANIFEST --out DIRECTORY [--scale 1..8]\n       keeperfx-frame-replay --sequence-fixture DIRECTORY")?;
    if input == "--sequence-fixture" {
        let directory = PathBuf::from(args.next().context("fixture output directory required")?);
        ensure!(args.next().is_none(), "unexpected fixture arguments");
        sequence::fixture(&directory)?;
        println!("Created synthetic sequence in {}", directory.display());
        return Ok(());
    }
    if input == "--fixture" {
        let directory = PathBuf::from(args.next().context("fixture output directory required")?);
        ensure!(args.next().is_none(), "unexpected fixture arguments");
        ensure!(!directory.exists(), "fixture directory already exists");
        fs::create_dir_all(&directory)?;
        let frame = Frame::fixture();
        frame.save(&directory.join("frame.kfx"))?;
        frame::write_png(
            &directory.join("reference.png"),
            frame.width,
            frame.height,
            &frame.rgba(),
        )?;
        println!("Created synthetic fixture in {}", directory.display());
        return Ok(());
    }
    let is_sequence = input == "--sequence";
    let input = if is_sequence {
        PathBuf::from(args.next().context("sequence manifest required")?)
    } else {
        PathBuf::from(input)
    };
    let mut reference = input.with_file_name("reference.png");
    let mut output = None;
    let mut scale = 1;
    while let Some(arg) = args.next() {
        let value = args.next().context("option requires a value")?;
        if arg == "--out" {
            output = Some(PathBuf::from(value));
        } else if arg == "--reference" {
            ensure!(
                !is_sequence,
                "sequence references are specified in its manifest"
            );
            reference = PathBuf::from(value);
        } else if arg == "--scale" {
            scale = value.to_str().context("invalid scale")?.parse::<u32>()?;
        } else {
            bail!("unknown option: {}", arg.to_string_lossy());
        }
    }
    let output = output.context("--out DIRECTORY is required")?;
    ensure!(
        !output.exists(),
        "output directory already exists; choose a new directory to preserve previous comparisons"
    );
    ensure!((1..=8).contains(&scale), "scale must be between 1 and 8");
    if is_sequence {
        let entries = sequence::load(&input)?;
        fs::create_dir_all(&output)?;
        let mut renderer = pollster::block_on(Replay::new())?;
        let mut results = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let name = format!("frame-{index:04}");
            let result = replay(
                &mut renderer,
                &entry.frame,
                &entry.reference,
                &output.join(&name),
                scale,
            )
            .with_context(|| format!("sequence frame {index}"))?;
            results.push(
                json!({"index":index, "report":format!("{name}/report.html"),
                "source":entry.frame, "reference":entry.reference,
                "different_pixels":result.0,
                "capture_reference_different_pixels":result.1,
                "passed":result == (0, 0)}),
            );
        }
        let passed = results.iter().all(|result| result["passed"] == true);
        let summary = json!({"format":"KFXSEQ01", "source":input, "scale":scale,
            "passed":passed, "frame_count":results.len(), "frames":results,
            "resource_reuse":"one renderer; textures and bindings retained until dimensions change",
            "setup_ms": renderer.setup_ms});
        fs::write(
            output.join("sequence-report.json"),
            serde_json::to_vec_pretty(&summary)?,
        )?;
        let links = results.iter().enumerate().map(|(index, result)| format!(
            r#"<li><a href="frame-{index:04}/report.html">Frame {index}</a>: {} (GPU: {}, capture: {})</li>"#,
            if result["passed"] == true { "PASS" } else { "FAIL" },
            result["different_pixels"], result["capture_reference_different_pixels"]
        )).collect::<String>();
        fs::write(
            output.join("report.html"),
            format!(
                r#"<!doctype html><meta charset="utf-8"><title>Frame sequence replay</title><h1>Sequence: {}</h1><p>Exact RGBA comparisons in manifest order. One renderer retains GPU resources across frames.</p><ol>{links}</ol>"#,
                if passed { "PASS" } else { "FAIL" }
            ),
        )?;
        println!(
            "Sequence {}: {} frames; report: {}",
            if passed { "PASS" } else { "FAIL" },
            entries.len(),
            output.join("report.html").display()
        );
        ensure!(passed, "sequence pixel comparison failed");
        return Ok(());
    }
    let mut renderer = pollster::block_on(Replay::new())?;
    let (changed, capture_mismatch) = replay(&mut renderer, &input, &reference, &output, scale)?;
    ensure!(
        changed == 0 && capture_mismatch == 0,
        "pixel comparison failed"
    );
    Ok(())
}

fn replay(
    renderer: &mut Replay,
    input: &Path,
    reference: &Path,
    output: &Path,
    scale: u32,
) -> Result<(usize, usize)> {
    let frame = Frame::load(input)?;
    let (rw, rh, reference_pixels) = frame::read_png(reference)?;
    ensure!(
        (rw, rh) == (frame.width, frame.height),
        "reference dimensions do not match the captured frame"
    );
    let (capture_mismatch, _, _) = report::compare(&reference_pixels, &frame.rgba())?;
    let reference_pixels = frame::scale_rgba(&reference_pixels, rw, rh, scale)?;
    let start = Instant::now();
    let pixels = renderer.render(&frame, scale)?;
    let render_readback_ms = start.elapsed().as_secs_f64() * 1000.0;
    let metadata = json!({
        "source": input.to_string_lossy(), "scale":scale,
        "adapter":renderer.adapter,"backend":renderer.backend,
        "setup_ms":renderer.setup_ms,"render_readback_ms":render_readback_ms,
        "setup_scope":"shared renderer initialization; excluded from render_readback_ms",
        "capture_reference_different_pixels":capture_mismatch,
    });
    let changed = report::write(
        output,
        rw * scale,
        rh * scale,
        &reference_pixels,
        &pixels,
        metadata,
    )?;
    println!(
        "{}: {} pixels differ; capture/reference mismatches: {}; report: {}",
        if changed == 0 && capture_mismatch == 0 {
            "PASS"
        } else {
            "FAIL"
        },
        changed,
        capture_mismatch,
        output.join("report.html").display()
    );
    Ok((changed, capture_mismatch))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
