use crate::{
    gltf,
    model::{self, ModelAsset},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MODEL_FORMAT_VERSION: u32 = 1;

#[derive(Debug)]
pub(crate) struct ModelReport {
    pub exported: usize,
    pub cached: usize,
    pub failed: Vec<ModelFailure>,
    pub models: Vec<ModelManifestEntry>,
    pub output_directory: PathBuf,
}

#[derive(Debug)]
pub(crate) struct ModelFailure {
    pub source: String,
    pub error: String,
}

#[derive(Debug)]
pub(crate) struct GlbReport {
    pub exported: usize,
    pub cached: usize,
    pub failed: Vec<ModelFailure>,
    pub output_directory: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct ModelManifest {
    version: u32,
    studs_per_tile: f32,
    includes_materials: bool,
    dependency_fingerprint: String,
    sources: Vec<ModelSourceManifestEntry>,
    models: Vec<ModelManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ModelManifestEntry {
    pub hash: String,
    pub output: String,
    pub source: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ModelSourceManifestEntry {
    source: String,
    hash: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct FingerprintCache {
    version: u32,
    files: HashMap<String, FingerprintCacheEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FingerprintCacheEntry {
    length: u64,
    modified_nanos: Option<u128>,
    hash: String,
}

pub(crate) fn export_models(
    input: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
    assets_dir: &Path,
    output_dir: &Path,
    studs_per_tile: f32,
    includes_materials: bool,
) -> Result<ModelReport, String> {
    let download_dir = absolute_path(download_dir)?;
    let mesh_dir = absolute_path(mesh_dir)?;
    let assets_dir = absolute_path(assets_dir)?;
    let output_dir = absolute_path(output_dir)?;
    fs::create_dir_all(&mesh_dir).map_err(|error| {
        format!(
            "failed to create decoded mesh directory {}: {error}",
            mesh_dir.display()
        )
    })?;
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create model output directory {}: {error}",
            output_dir.display()
        )
    })?;

    let source_files = model::source_files(input)?
        .into_iter()
        .map(|path| absolute_path(&path))
        .collect::<Result<Vec<_>, _>>()?;
    let dependency_fingerprint = tree_fingerprint(
        &download_dir,
        &assets_dir,
        &mesh_dir.join("fingerprint.json"),
    )?;
    let source_entries = source_manifest_entries(&source_files);
    if let Some(source_entries) = &source_entries
        && let Some(models) = reusable_models(
            &output_dir,
            &dependency_fingerprint,
            source_entries,
            studs_per_tile,
            includes_materials,
        )
    {
        return Ok(ModelReport {
            exported: 0,
            cached: models.len(),
            failed: Vec::new(),
            models,
            output_directory: output_dir,
        });
    }
    let mut manifest_entries = Vec::new();
    let mut report = ModelReport {
        exported: 0,
        cached: 0,
        failed: Vec::new(),
        models: Vec::new(),
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
        let models =
            match model::parse_models(&source_path, &download_dir, &mesh_dir, studs_per_tile) {
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
                studs_per_tile,
            );
            let output_stem = model_output_stem(&model_hash, includes_materials);
            let output_name = format!("{output_stem}.gltf");
            let output_path = output_dir.join(&output_name);
            let buffer_output_path = output_dir
                .parent()
                .unwrap_or(&output_dir)
                .join("bin")
                .join(format!("{output_stem}.bin"));

            for warning in &model_asset.warnings {
                eprintln!(
                    "warning model {} in {}: {}",
                    model_asset.name,
                    source_path.display(),
                    warning
                );
            }
            if output_path.is_file() && buffer_output_path.is_file() {
                let manifest_entry = ModelManifestEntry {
                    hash: model_hash,
                    output: output_path.display().to_string(),
                    source: source_path.display().to_string(),
                    name: model_asset.name.clone(),
                };
                manifest_entries.push(manifest_entry.clone());
                report.models.push(manifest_entry);
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
                includes_materials,
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
            let manifest_entry = ModelManifestEntry {
                hash: model_hash,
                output: output_path.display().to_string(),
                source: source_path.display().to_string(),
                name: model_asset.name.clone(),
            };
            manifest_entries.push(manifest_entry.clone());
            report.models.push(manifest_entry);
            report.exported += 1;
        }
    }

    let manifest = ModelManifest {
        version: MODEL_FORMAT_VERSION,
        studs_per_tile,
        includes_materials,
        dependency_fingerprint,
        sources: source_entries.unwrap_or_default(),
        models: manifest_entries,
    };
    let manifest = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize model manifest: {error}"))?;
    fs::write(output_dir.join("manifest.json"), format!("{manifest}\n"))
        .map_err(|error| format!("failed to write model manifest: {error}"))?;

    Ok(report)
}

pub(crate) fn export_glbs(
    models: &[ModelManifestEntry],
    output_dir: &Path,
) -> Result<GlbReport, String> {
    let output_dir = absolute_path(output_dir)?;
    fs::create_dir_all(&output_dir).map_err(|error| {
        format!(
            "failed to create GLB output directory {}: {error}",
            output_dir.display()
        )
    })?;
    let mut report = GlbReport {
        exported: 0,
        cached: 0,
        failed: Vec::new(),
        output_directory: output_dir.to_owned(),
    };

    for model in models {
        let output_stem = Path::new(&model.output)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("invalid GLTF output path: {}", model.output))?;
        let output_path = output_dir.join(format!("{output_stem}.glb"));
        if output_path.is_file() {
            report.cached += 1;
            continue;
        }

        let gltf_path = Path::new(&model.output);
        let gltf = match fs::read(gltf_path) {
            Ok(gltf) => gltf,
            Err(error) => {
                report.failed.push(ModelFailure {
                    source: model.source.clone(),
                    error: format!("failed to read GLTF {}: {error}", gltf_path.display()),
                });
                continue;
            }
        };
        let glb = match gltf::gltf_to_glb(&gltf, gltf_path) {
            Ok(glb) => glb,
            Err(error) => {
                report.failed.push(ModelFailure {
                    source: model.source.clone(),
                    error,
                });
                continue;
            }
        };
        if let Err(error) = fs::write(&output_path, glb) {
            report.failed.push(ModelFailure {
                source: model.source.clone(),
                error: format!("failed to write {}: {error}", output_path.display()),
            });
            continue;
        }
        report.exported += 1;
    }

    Ok(report)
}

fn absolute_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Ok(path.to_owned());
    }

    std::env::current_dir()
        .map(|current_dir| current_dir.join(path))
        .map_err(|error| format!("failed to determine the current working directory: {error}"))
}

fn model_fingerprint(
    source_bytes: &[u8],
    dependency_fingerprint: &str,
    model: &ModelAsset,
    model_index: usize,
    studs_per_tile: f32,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("roform-model-v{MODEL_FORMAT_VERSION}").as_bytes());
    hasher.update(source_bytes);
    hasher.update(dependency_fingerprint.as_bytes());
    hasher.update(model.name.as_bytes());
    hasher.update(model_index.to_string().as_bytes());
    hasher.update(model.primitives.len().to_string().as_bytes());
    hasher.update(&studs_per_tile.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

fn model_output_stem(model_hash: &str, includes_materials: bool) -> String {
    let prefix = if includes_materials { "" } else { "NM" };
    format!("{prefix}{model_hash}")
}

fn source_manifest_entries(paths: &[PathBuf]) -> Option<Vec<ModelSourceManifestEntry>> {
    paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).ok()?;
            Some(ModelSourceManifestEntry {
                source: path.display().to_string(),
                hash: blake3::hash(&bytes).to_hex().to_string(),
            })
        })
        .collect()
}

fn reusable_models(
    output_dir: &Path,
    dependency_fingerprint: &str,
    sources: &[ModelSourceManifestEntry],
    studs_per_tile: f32,
    includes_materials: bool,
) -> Option<Vec<ModelManifestEntry>> {
    let manifest_bytes = fs::read(output_dir.join("manifest.json")).ok()?;
    let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes).ok()?;
    if manifest.version != MODEL_FORMAT_VERSION
        || manifest.studs_per_tile != studs_per_tile
        || manifest.includes_materials != includes_materials
        || manifest.dependency_fingerprint != dependency_fingerprint
        || manifest.sources != sources
    {
        return None;
    }

    let buffer_dir = output_dir.parent().unwrap_or(output_dir).join("bin");
    if manifest.models.iter().all(|model| {
        let Some(output_stem) = Path::new(&model.output)
            .file_stem()
            .and_then(|stem| stem.to_str())
        else {
            return false;
        };
        Path::new(&model.output).is_file()
            && buffer_dir.join(format!("{output_stem}.bin")).is_file()
    }) {
        Some(manifest.models)
    } else {
        None
    }
}

fn tree_fingerprint(first: &Path, second: &Path, cache_path: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(first, &mut files)?;
    collect_files(second, &mut files)?;
    files.sort();

    let previous_cache = fs::read(cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<FingerprintCache>(&bytes).ok())
        .filter(|cache| cache.version == 1);
    let mut current_cache = HashMap::with_capacity(files.len());
    let mut hasher = blake3::Hasher::new();
    for path in files {
        let path_string = path.to_string_lossy().into_owned();
        let metadata = fs::metadata(&path)
            .map_err(|error| format!("failed to inspect dependency {}: {error}", path.display()))?;
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        let hash = if let Some(entry) = previous_cache
            .as_ref()
            .and_then(|cache| cache.files.get(&path_string))
            .filter(|entry| {
                modified_nanos.is_some()
                    && entry.length == metadata.len()
                    && entry.modified_nanos == modified_nanos
            }) {
            entry.hash.clone()
        } else {
            let bytes = fs::read(&path).map_err(|error| {
                format!("failed to read dependency {}: {error}", path.display())
            })?;
            blake3::hash(&bytes).to_hex().to_string()
        };

        hasher.update(path_string.as_bytes());
        hasher.update(hash.as_bytes());
        current_cache.insert(
            path_string,
            FingerprintCacheEntry {
                length: metadata.len(),
                modified_nanos,
                hash,
            },
        );
    }

    let cache = FingerprintCache {
        version: 1,
        files: current_cache,
    };
    if let Ok(bytes) = serde_json::to_vec(&cache)
        && let Err(error) = fs::write(cache_path, bytes)
    {
        eprintln!(
            "warning: failed to write dependency fingerprint cache {}: {error}",
            cache_path.display()
        );
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
