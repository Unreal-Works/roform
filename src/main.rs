mod csg;
mod gltf;
mod model;

use clap::Parser;
use mhif::{DownloadOptions, download_assets, extract_asset_ids_cached};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

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

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }

    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .map_err(|error| format!("failed to determine the current working directory: {error}"))
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.input, cli.out_dir, cli.assets_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: PathBuf, out_dir: PathBuf, assets_dir: PathBuf) -> Result<(), String> {
    // Download step
    let download_start_time = std::time::Instant::now();
    let download_out_dir: PathBuf = out_dir.join("download");
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

    // Model GLTF export step
    let model_start_time = std::time::Instant::now();
    let model_out_dir = out_dir.join("model");
    let model_report = export_models(&input, &download_out_dir, &assets_dir, &model_out_dir)?;
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

    Ok(())
}

fn decode_mesh_payload(bytes: &[u8]) -> Result<csg::UnionMesh, String> {
    let version = csg::payload_version(bytes);
    match version.as_str() {
        "CSGK" | "CSGMDL2" | "CSGMDL4" | "CSGMDL5" => {
            csg::decode_union_graphics(bytes).map_err(|error| format!("{version}: {error}"))
        }
        _ => csg::decode_mesh(bytes).map_err(|error| format!("{version}: {error}")),
    }
}

#[derive(Debug)]
struct ModelReport {
    exported: usize,
    cached: usize,
    failed: Vec<ModelFailure>,
    output_directory: PathBuf,
}

#[derive(Debug)]
struct ModelFailure {
    source: String,
    error: String,
}

#[derive(Debug, Serialize)]
struct ModelManifest {
    version: u32,
    models: Vec<ModelManifestEntry>,
}

#[derive(Debug, Serialize)]
struct ModelManifestEntry {
    hash: String,
    output: String,
    source: String,
    name: String,
}

fn export_models(
    input: &Path,
    download_dir: &Path,
    assets_dir: &Path,
    output_dir: &Path,
) -> Result<ModelReport, String> {
    let download_dir = absolute_path(download_dir)?;
    let assets_dir = absolute_path(assets_dir)?;
    let output_dir = absolute_path(output_dir)?;
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create model output directory {}: {error}",
            output_dir.display()
        )
    })?;

    let source_files = model::source_files(input)?;
    let dependency_fingerprint = tree_fingerprint(&download_dir, &assets_dir)?;
    let mut manifest_entries = Vec::new();
    let mut report = ModelReport {
        exported: 0,
        cached: 0,
        failed: Vec::new(),
        output_directory: output_dir.to_owned(),
    };

    for source_path in source_files {
        let source_path = absolute_path(&source_path)?;
        let source_bytes = match fs::read(&source_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.failed.push(ModelFailure {
                    source: source_path.display().to_string(),
                    error: format!("failed to read source: {error}"),
                });
                continue;
            }
        };
        let models = match model::parse_models(&source_path, &download_dir) {
            Ok(models) => models,
            Err(error) => {
                report.failed.push(ModelFailure {
                    source: source_path.display().to_string(),
                    error,
                });
                continue;
            }
        };
        for (model_index, model_asset) in models.into_iter().enumerate() {
            let model_hash = model_fingerprint(
                &source_bytes,
                &dependency_fingerprint,
                &model_asset,
                model_index,
            );
            let output_name = format!("{model_hash}.gltf");
            let output_path = output_dir.join(&output_name);
            let buffer_output_path = output_dir
                .parent()
                .unwrap_or(&output_dir)
                .join("bin")
                .join(format!("{model_hash}.bin"));

            for warning in &model_asset.warnings {
                eprintln!(
                    "warning model {} in {}: {}",
                    model_asset.name,
                    source_path.display(),
                    warning
                );
            }
            if output_path.is_file() && buffer_output_path.is_file() {
                manifest_entries.push(ModelManifestEntry {
                    hash: model_hash,
                    output: output_path.display().to_string(),
                    source: source_path.display().to_string(),
                    name: model_asset.name.clone(),
                });
                report.cached += 1;
                continue;
            }

            let gltf = match gltf::model_to_gltf(
                &model_asset,
                &download_dir,
                &assets_dir,
                &output_dir,
                output_dir.parent().unwrap_or(&output_dir),
                &buffer_output_path,
            ) {
                Ok(gltf) => gltf,
                Err(error) => {
                    report.failed.push(ModelFailure {
                        source: format!("{} / {}", source_path.display(), model_asset.name),
                        error,
                    });
                    continue;
                }
            };
            if let Err(error) = fs::write(&output_path, gltf) {
                report.failed.push(ModelFailure {
                    source: format!("{} / {}", source_path.display(), model_asset.name),
                    error: format!("failed to write {}: {error}", output_path.display()),
                });
                continue;
            }
            manifest_entries.push(ModelManifestEntry {
                hash: model_hash,
                output: output_path.display().to_string(),
                source: source_path.display().to_string(),
                name: model_asset.name.clone(),
            });
            report.exported += 1;
        }
    }

    let manifest = ModelManifest {
        version: 1,
        models: manifest_entries,
    };
    let manifest = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize model manifest: {error}"))?;
    fs::write(output_dir.join("manifest.json"), format!("{manifest}\n"))
        .map_err(|error| format!("failed to write model manifest: {error}"))?;

    Ok(report)
}

fn model_fingerprint(
    source_bytes: &[u8],
    dependency_fingerprint: &str,
    model: &model::ModelAsset,
    model_index: usize,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"roform-model-v2");
    hasher.update(source_bytes);
    hasher.update(dependency_fingerprint.as_bytes());
    hasher.update(model.name.as_bytes());
    hasher.update(model_index.to_string().as_bytes());
    hasher.update(model.primitives.len().to_string().as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn tree_fingerprint(first: &Path, second: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(first, &mut files)?;
    collect_files(second, &mut files)?;
    files.sort();

    let mut hasher = blake3::Hasher::new();
    for path in files {
        hasher.update(path.to_string_lossy().as_bytes());
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read dependency {}: {error}", path.display()))?;
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        files.push(path.to_owned());
        return Ok(());
    }
    if !path.is_dir() {
        return Ok(());
    }
    let entries = fs::read_dir(path).map_err(|error| {
        format!(
            "failed to read dependency directory {}: {error}",
            path.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect dependency directory {}: {error}",
                path.display()
            )
        })?;
        collect_files(&entry.path(), files)?;
    }
    Ok(())
}
