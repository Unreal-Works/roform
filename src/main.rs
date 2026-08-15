mod csg;
mod gltf;

use clap::Parser;
use mhif::{DownloadOptions, download_assets, extract_asset_ids_cached};
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
}

fn default_output_dir() -> PathBuf {
    std::env::current_dir()
        .expect("failed to determine the current working directory")
        .join("roform")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.input, cli.out_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(input: PathBuf, out_dir: PathBuf) -> Result<(), String> {
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
        let fingerprint_path = output_dir.join(format!("{asset_id}.blake3"));
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
