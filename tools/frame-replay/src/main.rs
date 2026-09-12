mod frame;
mod gpu;
mod report;

use anyhow::{Context, Result, bail, ensure};
use frame::Frame;
use serde_json::json;
use std::{env, fs, path::PathBuf};

fn run() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let input = args.next().context("usage: keeperfx-frame-replay FRAME.kfx --out DIRECTORY [--reference PNG] [--scale 1..8]\n       keeperfx-frame-replay --fixture DIRECTORY")?;
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
    let input = PathBuf::from(input);
    let mut reference = input.with_file_name("reference.png");
    let mut output = None;
    let mut scale = 1;
    while let Some(arg) = args.next() {
        let value = args.next().context("option requires a value")?;
        if arg == "--out" {
            output = Some(PathBuf::from(value));
        } else if arg == "--reference" {
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
    let frame = Frame::load(&input)?;
    let (rw, rh, reference_pixels) = frame::read_png(&reference)?;
    ensure!(
        (rw, rh) == (frame.width, frame.height),
        "reference dimensions do not match the captured frame"
    );
    let (capture_mismatch, _, _) = report::compare(&reference_pixels, &frame.rgba())?;
    let reference_pixels = frame::scale_rgba(&reference_pixels, rw, rh, scale)?;
    let rendered = pollster::block_on(gpu::render(&frame, scale))?;
    let metadata = json!({
        "source": input.to_string_lossy(), "scale":scale,
        "adapter":rendered.adapter,"backend":rendered.backend,
        "setup_ms":rendered.setup_ms,"render_readback_ms":rendered.render_readback_ms,
        "capture_reference_different_pixels":capture_mismatch,
    });
    let changed = report::write(
        &output,
        rw * scale,
        rh * scale,
        &reference_pixels,
        &rendered.pixels,
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
    ensure!(
        changed == 0 && capture_mismatch == 0,
        "pixel comparison failed"
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
