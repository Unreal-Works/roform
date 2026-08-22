use clap::{Parser, ValueEnum};
use mhif::{DEFAULT_DOWNLOAD_JOBS, DownloadOptions, download_assets, extract_asset_ids_cached};
use roform::{
    ModelExportOptions, export_glbs_with_jobs, export_meshes_with_jobs, export_models_with_jobs,
};
use std::{path::PathBuf, process::ExitCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CompileStage {
    Mesh,
    Model,
    Glb,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CompilePlan {
    mesh: bool,
    model: bool,
    glb: bool,
}

fn compile_plan(stages: &[CompileStage], no_compile: bool) -> CompilePlan {
    if no_compile {
        return CompilePlan::default();
    }

    let glb = stages.contains(&CompileStage::Glb);
    let model = stages.contains(&CompileStage::Model) || glb;
    let mesh = stages.contains(&CompileStage::Mesh) || model;
    CompilePlan { mesh, model, glb }
}

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
    #[arg(
        long,
        value_delimiter = ',',
        value_name = "STAGE[,STAGE...]",
        default_values = ["mesh", "model"],
        conflicts_with = "no_compile",
        help = "Stages to compile: mesh, model, or glb (default: mesh,model)"
    )]
    compile: Vec<CompileStage>,
    #[arg(
        long,
        conflicts_with = "compile",
        help = "Skip mesh, model, and GLB compilation"
    )]
    no_compile: bool,
    #[arg(
        long,
        value_name = "DIRECTORY",
        help = "Directory containing fallback material images; enables materials"
    )]
    materials_dir: Option<PathBuf>,
    #[arg(
        long,
        help = "Ignore cached model outputs and re-export GLTF and GLB files"
    )]
    recompile: bool,
    #[arg(
        long,
        value_name = "N",
        default_value_t = DEFAULT_DOWNLOAD_JOBS,
        help = "Maximum number of concurrent downloads and compile workers"
    )]
    jobs: usize,
    #[arg(
        long,
        value_name = "STUDS",
        default_value_t = 2.0,
        help = "Physical studs represented by one texture tile"
    )]
    studs_per_tile: f32,
}

fn default_output_dir() -> PathBuf {
    std::env::current_dir()
        .expect("failed to determine the current working directory")
        .join("roform")
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(
        cli.input,
        cli.out_dir,
        compile_plan(&cli.compile, cli.no_compile),
        cli.materials_dir,
        cli.recompile,
        cli.studs_per_tile,
        cli.jobs,
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
    compile: CompilePlan,
    materials_dir: Option<PathBuf>,
    recompile: bool,
    studs_per_tile: f32,
    jobs: usize,
) -> Result<(), String> {
    if !studs_per_tile.is_finite() || studs_per_tile <= 0.0 {
        return Err("--studs-per-tile must be finite and greater than zero".to_owned());
    }

    let includes_materials = materials_dir.is_some();
    let materials_dir = materials_dir.unwrap_or_default();
    let download_start_time = std::time::Instant::now();
    let download_out_dir = out_dir.join("download");
    let extraction =
        extract_asset_ids_cached(&input, &download_out_dir).map_err(|error| error.to_string())?;
    let download_report = download_assets(
        &extraction.asset_ids,
        &DownloadOptions::new(&download_out_dir).with_jobs(jobs),
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

    if compile.mesh {
        let mesh_start_time = std::time::Instant::now();
        let mesh_report =
            export_meshes_with_jobs(&input, &download_out_dir, &out_dir.join("mesh"), jobs)?;
        for failure in &mesh_report.failed {
            eprintln!("failed mesh {}: {}", failure.source, failure.error);
        }
        println!(
            "mesh: decoded {}, reused {}, failed {} -> {} in {:.2}s",
            mesh_report.decoded,
            mesh_report.cached,
            mesh_report.failed.len(),
            mesh_report.output_directory.display(),
            mesh_start_time.elapsed().as_secs_f64()
        );
    }

    let model_report = if compile.model {
        let model_start_time = std::time::Instant::now();
        let model_report = export_models_with_jobs(
            &input,
            &download_out_dir,
            &out_dir.join("mesh"),
            &materials_dir,
            &out_dir.join("model"),
            ModelExportOptions {
                studs_per_tile,
                includes_materials,
                recompile,
            },
            jobs,
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
        Some(model_report)
    } else {
        None
    };

    if compile.glb {
        let glb_start_time = std::time::Instant::now();
        let models = model_report
            .as_ref()
            .ok_or_else(|| "GLB compilation requires model compilation".to_owned())?;
        let glb_report =
            export_glbs_with_jobs(&models.models, &out_dir.join("glb"), recompile, jobs)?;
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
    fn materials_are_enabled_only_when_a_directory_is_provided() {
        let no_materials_cli = Cli::try_parse_from(["roform", "input.rbxmx"]).unwrap();
        assert!(no_materials_cli.materials_dir.is_none());

        let materials_cli =
            Cli::try_parse_from(["roform", "input.rbxmx", "--materials-dir", "materials"]).unwrap();
        assert_eq!(
            materials_cli.materials_dir,
            Some(PathBuf::from("materials"))
        );

        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--assets-dir", "assets"]).is_err());
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--materials"]).is_err());
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--no-materials"]).is_err());

        let recompile_cli = Cli::try_parse_from(["roform", "input.rbxmx", "--recompile"]).unwrap();
        assert!(recompile_cli.recompile);

        let default_cli = Cli::try_parse_from(["roform", "input.rbxmx"]).unwrap();
        assert_eq!(default_cli.jobs, DEFAULT_DOWNLOAD_JOBS);
        assert_eq!(
            compile_plan(&default_cli.compile, default_cli.no_compile),
            CompilePlan {
                mesh: true,
                model: true,
                glb: false,
            }
        );

        let jobs_cli = Cli::try_parse_from(["roform", "input.rbxmx", "--jobs", "8"]).unwrap();
        assert_eq!(jobs_cli.jobs, 8);

        let mesh_cli = Cli::try_parse_from(["roform", "input.rbxmx", "--compile", "mesh"]).unwrap();
        assert_eq!(
            compile_plan(&mesh_cli.compile, mesh_cli.no_compile),
            CompilePlan {
                mesh: true,
                model: false,
                glb: false,
            }
        );
        assert_eq!(
            compile_plan(&[CompileStage::Model], false),
            CompilePlan {
                mesh: true,
                model: true,
                glb: false,
            }
        );
        assert_eq!(
            compile_plan(&[CompileStage::Glb], false),
            CompilePlan {
                mesh: true,
                model: true,
                glb: true,
            }
        );

        let all_cli =
            Cli::try_parse_from(["roform", "input.rbxmx", "--compile", "mesh,model,glb"]).unwrap();
        assert_eq!(
            compile_plan(&all_cli.compile, all_cli.no_compile),
            CompilePlan {
                mesh: true,
                model: true,
                glb: true,
            }
        );

        let no_compile_cli =
            Cli::try_parse_from(["roform", "input.rbxmx", "--no-compile"]).unwrap();
        assert_eq!(
            compile_plan(&no_compile_cli.compile, no_compile_cli.no_compile),
            CompilePlan::default()
        );
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--compile", "unknown"]).is_err());
        assert!(
            Cli::try_parse_from(["roform", "input.rbxmx", "--compile", "mesh", "--no-compile"])
                .is_err()
        );
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--model"]).is_err());
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--no-model"]).is_err());
        assert!(Cli::try_parse_from(["roform", "input.rbxmx", "--glb"]).is_err());
    }
}
