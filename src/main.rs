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

    // Mesh decoding step
    let mesh_start_time = std::time::Instant::now();
    let mesh_out_dir = out_dir.join("mesh");
    let mesh_report: MeshReport = decode_downloaded_meshes(&download_out_dir, &mesh_out_dir)?;
    for failure in &mesh_report.failed {
        eprintln!("failed mesh {}: {}", failure.asset_id, failure.error);
    }
    println!(
        "mesh: decoded {}, reused {}, failed {} -> {} in {:.2}s",
        mesh_report.decoded,
        mesh_report.cached,
        mesh_report.failed.len(),
        mesh_report.output_directory.display(),
        mesh_start_time.elapsed().as_secs_f64()
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
#[derive(Debug)]
struct MeshReport {
    decoded: usize,
    cached: usize,
    failed: Vec<MeshFailure>,
    output_directory: PathBuf,
}

#[derive(Debug)]
struct MeshFailure {
    asset_id: String,
    error: String,
}

fn decode_downloaded_meshes(download_dir: &Path, output_dir: &Path) -> Result<MeshReport, String> {
    fs::create_dir_all(output_dir).map_err(|error| {
        format!(
            "failed to create mesh output directory {}: {error}",
            output_dir.display()
        )
    })?;
    let fingerprint_dir = output_dir.join(".fingerprint");
    fs::create_dir_all(&fingerprint_dir).map_err(|error| {
        format!(
            "failed to create mesh fingerprint directory {}: {error}",
            fingerprint_dir.display()
        )
    })?;

    let mut report = MeshReport {
        decoded: 0,
        cached: 0,
        failed: Vec::new(),
        output_directory: output_dir.to_owned(),
    };
    let entries = fs::read_dir(download_dir).map_err(|error| {
        format!(
            "failed to read downloaded asset directory {}: {error}",
            download_dir.display()
        )
    })?;

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to inspect downloaded asset directory {}: {error}",
                download_dir.display()
            )
        })?;
        let asset_dir = entry.path();
        if !asset_dir.is_dir() {
            continue;
        }

        let asset_id = entry.file_name().to_string_lossy().into_owned();
        let payload_path = asset_dir.join("asset.bin");
        if !payload_path.is_file() {
            continue;
        }

        let bytes = match fs::read(&payload_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.failed.push(MeshFailure {
                    asset_id,
                    error: format!("failed to read {}: {error}", payload_path.display()),
                });
                continue;
            }
        };
        let fingerprint = blake3::hash(&bytes).to_hex().to_string();
        let output_path = output_dir.join(format!("{asset_id}.glb"));
        let fingerprint_path = fingerprint_dir.join(format!("{asset_id}.blake3"));
        if output_path.is_file()
            && fs::read_to_string(&fingerprint_path)
                .map(|cached_fingerprint| cached_fingerprint.trim() == fingerprint)
                .unwrap_or(false)
        {
            report.cached += 1;
            continue;
        }

        let mesh = match decode_mesh_payload(&bytes) {
            Ok(mesh) => mesh,
            Err(error) => {
                report.failed.push(MeshFailure { asset_id, error });
                continue;
            }
        };

        if let Err(error) = fs::write(&output_path, gltf::union_to_glb(&mesh)) {
            report.failed.push(MeshFailure {
                asset_id,
                error: format!("failed to write {}: {error}", output_path.display()),
            });
            continue;
        }
        if let Err(error) = fs::write(&fingerprint_path, fingerprint) {
            report.failed.push(MeshFailure {
                asset_id,
                error: format!("failed to write {}: {error}", fingerprint_path.display()),
            });
            continue;
        }
        report.decoded += 1;
    }

    Ok(report)
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
    fs::create_dir_all(output_dir).map_err(|error| {
        format!(
            "failed to create model output directory {}: {error}",
            output_dir.display()
        )
    })?;

    let source_files = model::source_files(input)?;
    let dependency_fingerprint = tree_fingerprint(download_dir, assets_dir)?;
    let mut manifest_entries = Vec::new();
    let mut report = ModelReport {
        exported: 0,
        cached: 0,
        failed: Vec::new(),
        output_directory: output_dir.to_owned(),
    };

    for source_path in source_files {
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
        let models = match model::parse_models(&source_path, download_dir) {
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

            for warning in &model_asset.warnings {
                eprintln!(
                    "warning model {} in {}: {}",
                    model_asset.name,
                    source_path.display(),
                    warning
                );
            }
            if output_path.is_file() {
                manifest_entries.push(ModelManifestEntry {
                    hash: model_hash,
                    output: output_name,
                    source: source_path.display().to_string(),
                    name: model_asset.name.clone(),
                });
                report.cached += 1;
                continue;
            }

            let gltf = match gltf::model_to_gltf(&model_asset, download_dir, assets_dir) {
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
                output: output_name,
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
    hasher.update(b"roform-model-v1");
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
