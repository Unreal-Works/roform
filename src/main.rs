use clap::{ArgAction, Parser};
use mhif::{DownloadOptions, download_assets, extract_asset_ids_cached};
use roform::{ModelExportOptions, export_glbs, export_models};
use std::{path::PathBuf, process::ExitCode};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Download Roblox assets referenced by a file or directory"
)]
struct Cli {
    #[arg(
        value_name = "INPUT",
        help = "File or directory to scan for asset references"
    )]
    input: PathBuf,
    #[arg(long, value_name = "DIRECTORY", default_value_os_t = default_output_dir())]
    out_dir: PathBuf,
    #[arg(long, value_name = "DIRECTORY", default_value_os_t = default_assets_dir())]
    assets_dir: PathBuf,
    #[arg(long, help = "Also package each exported GLTF as a GLB file")]
    glb: bool,
    #[arg(
        long,
        help = "Ignore cached model outputs and re-export GLTF and GLB files"
    )]
    recompile: bool,
    #[arg(
        long = "no-materials",
        default_value_t = true,
        action = ArgAction::SetFalse,
        help = "Do not assign materials to exported geometry"
    )]
    materials: bool,
    #[arg(
        long,
        value_name = "STUDS",
        default_value_t = 1.0,
        help = "Physical studs represented by one texture tile"
    )]
    studs_per_tile: f32,
}

fn default_output_dir() -> PathBuf {
    std::env::current_dir()
        .expect("failed to determine the current working directory")
        .join("roform")
}

fn default_assets_dir() -> PathBuf {
    std::env::current_dir()
        .expect("failed to determine the current working directory")
        .join("assets")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(
        cli.input,
        cli.out_dir,
        cli.assets_dir,
        cli.glb,
        cli.recompile,
        cli.materials,
        cli.studs_per_tile,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(
    input: PathBuf,
    out_dir: PathBuf,
    assets_dir: PathBuf,
    glb: bool,
    recompile: bool,
    materials: bool,
    studs_per_tile: f32,
) -> Result<(), String> {
    if !studs_per_tile.is_finite() || studs_per_tile <= 0.0 {
        return Err("--studs-per-tile must be finite and greater than zero".to_owned());
    }

    let download_start_time = std::time::Instant::now();
    let download_out_dir = out_dir.join("download");
    let extraction =
        extract_asset_ids_cached(&input, &download_out_dir).map_err(|error| error.to_string())?;
    let download_report = download_assets(
        &extraction.asset_ids,
        &DownloadOptions::new(&download_out_dir),
    )
    .map_err(|error| error.to_string())?;
    for failure in &download_report.failed {
        eprintln!("failed {}: {}", failure.asset_id, failure.error);
    }
    println!(
        "download: fetched {}, reused {}, failed {} -> {} in {:.2}s",
        download_report.downloaded,
        download_report.cached,
        download_report.failed.len(),
        download_report.output_directory.display(),
        download_start_time.elapsed().as_secs_f64()
    );

    let model_start_time = std::time::Instant::now();
    let model_report = export_models(
        &input,
        &download_out_dir,
        &out_dir.join("mesh"),
        &assets_dir,
        &out_dir.join("model"),
        ModelExportOptions {
            studs_per_tile,
            includes_materials: materials,
            recompile,
        },
    )?;
    for failure in &model_report.failed {
        eprintln!("failed model {}: {}", failure.source, failure.error);
    }
    println!(
        "model: exported {}, reused {}, failed {} -> {} in {:.2}s",
        model_report.exported,
        model_report.cached,
        model_report.failed.len(),
        model_report.output_directory.display(),
        model_start_time.elapsed().as_secs_f64()
    );

    if glb {
        let glb_start_time = std::time::Instant::now();
        let glb_report = export_glbs(&model_report.models, &out_dir.join("glb"), recompile)?;
        println!(
            "glb: exported {}, reused {}, failed {} -> {} in {:.2}s",
            glb_report.exported,
            glb_report.cached,
            glb_report.failed.len(),
            glb_report.output_directory.display(),
            glb_start_time.elapsed().as_secs_f64()
        );
        for failure in &glb_report.failed {
            eprintln!("failed GLB {}: {}", failure.source, failure.error);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn materials_are_enabled_by_default_and_disabled_by_flag() {
        let default_cli = Cli::try_parse_from(["roform", "input.rbxmx"]).unwrap();
        assert!(default_cli.materials);

        let no_materials_cli =
            Cli::try_parse_from(["roform", "input.rbxmx", "--no-materials"]).unwrap();
        assert!(!no_materials_cli.materials);

        let recompile_cli = Cli::try_parse_from(["roform", "input.rbxmx", "--recompile"]).unwrap();
        assert!(recompile_cli.recompile);
    }
}
