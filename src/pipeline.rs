use crate::{
    gltf,
    model::{self, ModelAsset},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MODEL_FORMAT_VERSION: u32 = 1;
const MISSING_DEPENDENCY_HASH: &str = "<missing>";

/// The result of exporting the models found in an input path.
#[derive(Debug)]
pub struct ModelReport {
    /// Number of models written as new GLTF files during this call.
    pub exported: usize,
    /// Number of models reused from the model cache during this call.
    pub cached: usize,
    /// Models that could not be exported. These failures are accumulated so
    /// that other sources can still be processed.
    pub failed: Vec<ModelFailure>,
    /// Successfully exported or reused models for the requested input.
    pub models: Vec<ModelManifestEntry>,
    /// Absolute directory containing the model GLTF files and manifest.
    pub output_directory: PathBuf,
}

/// A model that failed during an export stage.
#[derive(Debug)]
pub struct ModelFailure {
    /// Source file or model name associated with the failure.
    pub source: String,
    /// Human-readable description of the failure.
    pub error: String,
}

/// The result of packaging model GLTF files as GLB files.
#[derive(Debug)]
pub struct GlbReport {
    /// Number of GLB files written as new files during this call.
    pub exported: usize,
    /// Number of GLB files reused from the output directory during this call.
    pub cached: usize,
    /// Models whose GLB files could not be packaged. These failures are
    /// accumulated so that other models can still be processed.
    pub failed: Vec<ModelFailure>,
    /// Absolute directory containing the GLB files.
    pub output_directory: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
struct ModelManifest {
    version: u32,
    studs_per_tile: f32,
    includes_materials: bool,
    dependencies: BTreeMap<String, String>,
    sources: Vec<ModelSourceManifestEntry>,
    models: Vec<ModelManifestEntry>,
}

/// Metadata for a model GLTF file produced by [`export_models`].
///
/// Entries returned by [`ModelReport::models`] can be passed to
/// [`export_glbs`]. The paths are absolute and use forward slashes so they can
/// be stored consistently in the manifest on every platform.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelManifestEntry {
    /// Content and configuration hash used as the model output stem.
    pub hash: String,
    /// Absolute path to the model GLTF file.
    pub output: String,
    /// Absolute path to the Roblox source file.
    pub source: String,
    /// Name of the Roblox model represented by the file.
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ModelSourceManifestEntry {
    source: String,
    hash: String,
    dependencies: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct FingerprintCache {
    version: u32,
    files: HashMap<String, FingerprintCacheEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FingerprintCacheEntry {
    length: u64,
    modified_nanos: Option<u128>,
    hash: String,
}

/// Options controlling model GLTF export.
#[derive(Clone, Copy, Debug)]
pub struct ModelExportOptions {
    /// Number of Roblox studs represented by one texture tile.
    ///
    /// This value must be finite and greater than zero. The default is `1.0`.
    pub studs_per_tile: f32,
    /// Whether material definitions and available material textures are
    /// included in the exported GLTF. The default is `true`.
    pub includes_materials: bool,
    /// Whether decoded meshes should also be compiled into model GLTF files.
    /// Meshes are decoded and cached even when this is `false`. The default is
    /// `true`.
    pub compile_models: bool,
    /// Whether existing model GLTF files should be ignored and regenerated.
    /// Cached downloads and decoded meshes are still reusable. The default is
    /// `false`.
    pub recompile: bool,
}

impl Default for ModelExportOptions {
    fn default() -> Self {
        Self {
            studs_per_tile: 1.0,
            includes_materials: true,
            compile_models: true,
            recompile: false,
        }
    }
}

impl ModelExportOptions {
    fn validate(self) -> Result<(), String> {
        if !self.studs_per_tile.is_finite() || self.studs_per_tile <= 0.0 {
            return Err("studs_per_tile must be finite and greater than zero".to_owned());
        }
        Ok(())
    }
}

/// Export the Roblox models found at `input` as GLTF files.
///
/// `input` may be a single `.rbxm`, `.rbxmx`, `.rbxl`, or `.rbxlx` file, or a
/// directory that is searched recursively for those extensions. Relative paths
/// are resolved against the current working directory. `download_dir` must
/// contain downloaded Roblox assets referenced by the input, and `materials_dir`
/// may contain fallback material images. `mesh_dir` stores decoded mesh cache
/// files and `output_dir` stores model GLTF files, external buffers, and the
/// model manifest when `options.compile_models` is `true`. Meshes are still
/// decoded when model compilation is disabled.
///
/// A successful result can still contain entries in [`ModelReport::failed`].
/// Per-source and per-model failures are reported there while processing
/// continues; failures to prepare the export directories, serialize the
/// manifest, or write model output are returned as `Err`. Dependency
/// fingerprint cache write failures are reported as warnings because the
/// cache can be rebuilt on the next call.
pub fn export_models(
    input: &Path,
    download_dir: &Path,
    mesh_dir: &Path,
    materials_dir: &Path,
    output_dir: &Path,
    options: ModelExportOptions,
) -> Result<ModelReport, String> {
    options.validate()?;
    let ModelExportOptions {
        studs_per_tile,
        includes_materials,
        compile_models,
        recompile,
    } = options;
    let download_dir = absolute_path(download_dir)?;
    let mesh_dir = absolute_path(mesh_dir)?;
    let materials_dir = absolute_path(materials_dir)?;
    let output_dir = absolute_path(output_dir)?;
    fs::create_dir_all(&mesh_dir).map_err(|error| {
        format!(
            "failed to create decoded mesh directory {}: {error}",
            mesh_dir.display()
        )
    })?;
    if compile_models {
        fs::create_dir_all(&output_dir).map_err(|error| {
            format!(
                "failed to create model output directory {}: {error}",
                output_dir.display()
            )
        })?;
    }

    let source_files = model::source_files(input)?
        .into_iter()
        .map(|path| absolute_path(&path))
        .collect::<Result<Vec<_>, _>>()?;
    let mut report = ModelReport {
        exported: 0,
        cached: 0,
        failed: Vec::new(),
        models: Vec::new(),
        output_directory: output_dir.to_owned(),
    };
    if !compile_models {
        for source_path in source_files {
            match model::parse_models(&source_path, &download_dir, &mesh_dir, studs_per_tile) {
                Ok(models) => {
                    for model_asset in models {
                        for warning in &model_asset.warnings {
                            eprintln!(
                                "warning model {} in {}: {}",
                                model_asset.name,
                                source_path.display(),
                                warning
                            );
                        }
                    }
                }
                Err(error) => report.failed.push(ModelFailure {
                    source: source_path.display().to_string(),
                    error,
                }),
            }
        }
        return Ok(report);
    }
    let source_entries = source_manifest_entries(&source_files).unwrap_or_default();
    let source_hashes = source_entries
        .iter()
        .map(|entry| (entry.source.clone(), entry.hash.clone()))
        .collect::<HashMap<_, _>>();
    let previous_manifest = if recompile {
        None
    } else {
        reusable_manifest(&output_dir, studs_per_tile, includes_materials)
    };
    let mut fingerprints = FingerprintState::load(&mesh_dir.join("fingerprint.json"));
    let current_sources = source_files
        .iter()
        .map(|path| cache_path(path))
        .collect::<HashSet<_>>();
    let mut manifest_entries = previous_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .models
                .iter()
                .filter(|model| !current_sources.contains(&model.source))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut manifest_sources = previous_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .sources
                .iter()
                .filter(|source| !current_sources.contains(&source.source))
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut manifest_dependencies = previous_manifest
        .as_ref()
        .map(|manifest| manifest.dependencies.clone())
        .unwrap_or_default();
    for source_path in source_files {
        let source = cache_path(&source_path);
        if let Some(source_hash) = source_hashes.get(&source)
            && let Some(previous_manifest) = &previous_manifest
            && let Some((models, dependencies)) = reusable_source_models(
                previous_manifest,
                &source,
                source_hash,
                &mut fingerprints,
                &output_dir,
            )
        {
            manifest_dependencies.extend(dependencies.clone());
            manifest_sources.push(ModelSourceManifestEntry {
                source,
                hash: source_hash.clone(),
                dependencies: dependencies.keys().cloned().collect(),
            });
            report.cached += models.len();
            manifest_entries.extend(models.iter().cloned());
            report.models.extend(models);
            continue;
        }

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
        let dependencies = source_dependencies(
            &models,
            &download_dir,
            &materials_dir,
            includes_materials,
            &mut fingerprints,
        )?;
        if let Some(source_hash) = source_hashes.get(&source) {
            manifest_dependencies.extend(dependencies.clone());
            manifest_sources.push(ModelSourceManifestEntry {
                source: source.clone(),
                hash: source_hash.clone(),
                dependencies: dependencies.keys().cloned().collect(),
            });
        }
        for (model_index, model_asset) in models.into_iter().enumerate() {
            let model_hash = model_fingerprint(
                &source_bytes,
                &model_asset,
                model_index,
                studs_per_tile,
                ModelFingerprintContext {
                    download_dir: &download_dir,
                    materials_dir: &materials_dir,
                    includes_materials,
                    fingerprints: &mut fingerprints,
                },
            )?;
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
            if !recompile && output_path.is_file() && buffer_output_path.is_file() {
                let manifest_entry = ModelManifestEntry {
                    hash: model_hash,
                    output: cache_path(&output_path),
                    source: source.clone(),
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
                &materials_dir,
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
                output: cache_path(&output_path),
                source: source.clone(),
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
        dependencies: manifest_dependencies,
        sources: manifest_sources,
        models: manifest_entries,
    };
    let manifest = serde_json::to_string_pretty(&manifest)
        .map_err(|error| format!("failed to serialize model manifest: {error}"))?;
    fs::write(output_dir.join("manifest.json"), format!("{manifest}\n"))
        .map_err(|error| format!("failed to write model manifest: {error}"))?;
    fingerprints.write(&mesh_dir.join("fingerprint.json"));

    Ok(report)
}

/// Package model GLTF files as GLB files.
///
/// Pass the [`ModelManifestEntry`] values returned in [`ModelReport::models`]
/// by [`export_models`]. The GLTF paths in those entries are read as absolute
/// paths, while `output_dir` is resolved against the current working directory
/// when it is relative. Each GLB contains the external buffer and any images
/// referenced by its source GLTF.
///
/// Existing GLB files are reused unless `recompile` is `true`. A successful
/// result can still contain entries in [`GlbReport::failed`]; failures for one
/// model do not prevent the remaining models from being packaged.
pub fn export_glbs(
    models: &[ModelManifestEntry],
    output_dir: &Path,
    recompile: bool,
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
        if !recompile && output_path.is_file() {
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

fn cache_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn model_fingerprint(
    source_bytes: &[u8],
    model: &ModelAsset,
    model_index: usize,
    studs_per_tile: f32,
    context: ModelFingerprintContext<'_>,
) -> Result<String, String> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("roform-model-v{MODEL_FORMAT_VERSION}").as_bytes());
    hasher.update(source_bytes);
    hasher.update(model_index.to_string().as_bytes());
    hasher.update(&studs_per_tile.to_le_bytes());
    hasher.update(&[context.includes_materials as u8]);
    for path in model_dependency_paths(
        model,
        context.download_dir,
        context.materials_dir,
        context.includes_materials,
    ) {
        hasher.update(cache_path(&path).as_bytes());
        hasher.update(context.fingerprints.fingerprint(&path)?.as_bytes());
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn model_output_stem(model_hash: &str, includes_materials: bool) -> String {
    let prefix = if includes_materials { "M" } else { "" };
    format!("{prefix}{model_hash}")
}

fn source_manifest_entries(paths: &[PathBuf]) -> Option<Vec<ModelSourceManifestEntry>> {
    paths
        .iter()
        .map(|path| {
            let bytes = fs::read(path).ok()?;
            Some(ModelSourceManifestEntry {
                source: cache_path(path),
                hash: blake3::hash(&bytes).to_hex().to_string(),
                dependencies: Vec::new(),
            })
        })
        .collect()
}

fn reusable_manifest(
    output_dir: &Path,
    studs_per_tile: f32,
    includes_materials: bool,
) -> Option<ModelManifest> {
    let manifest_bytes = fs::read(output_dir.join("manifest.json")).ok()?;
    let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes).ok()?;
    if manifest.version != MODEL_FORMAT_VERSION
        || manifest.studs_per_tile != studs_per_tile
        || manifest.includes_materials != includes_materials
    {
        return None;
    }
    Some(normalize_manifest_paths(manifest))
}

fn normalize_manifest_paths(mut manifest: ModelManifest) -> ModelManifest {
    manifest.dependencies = manifest
        .dependencies
        .into_iter()
        .map(|(path, hash)| (cache_path(Path::new(&path)), hash))
        .collect();
    for source in &mut manifest.sources {
        source.source = cache_path(Path::new(&source.source));
        source.dependencies = source
            .dependencies
            .iter()
            .map(|path| cache_path(Path::new(path)))
            .collect();
    }
    for model in &mut manifest.models {
        model.output = cache_path(Path::new(&model.output));
        model.source = cache_path(Path::new(&model.source));
    }
    manifest
}

fn normalize_fingerprint_cache(mut cache: FingerprintCache) -> FingerprintCache {
    cache.files = cache
        .files
        .into_iter()
        .map(|(path, entry)| (cache_path(Path::new(&path)), entry))
        .collect();
    cache
}

fn reusable_source_models(
    manifest: &ModelManifest,
    source: &str,
    source_hash: &str,
    fingerprints: &mut FingerprintState,
    output_dir: &Path,
) -> Option<(Vec<ModelManifestEntry>, BTreeMap<String, String>)> {
    let source_manifest = manifest
        .sources
        .iter()
        .find(|entry| entry.source == source && entry.hash == source_hash)?;
    let dependencies = source_manifest
        .dependencies
        .iter()
        .map(|path| Some((path.clone(), manifest.dependencies.get(path)?.clone())))
        .collect::<Option<BTreeMap<_, _>>>()?;
    for (path, expected_hash) in &dependencies {
        let hash = fingerprints.fingerprint(Path::new(path)).ok()?;
        if hash != *expected_hash {
            return None;
        }
    }

    let buffer_dir = output_dir.parent().unwrap_or(output_dir).join("bin");
    let models = manifest
        .models
        .iter()
        .filter(|model| model.source == source)
        .cloned()
        .collect::<Vec<_>>();
    if models.is_empty() {
        return None;
    }
    if models.iter().all(|model| {
        let Some(output_stem) = Path::new(&model.output)
            .file_stem()
            .and_then(|stem| stem.to_str())
        else {
            return false;
        };
        Path::new(&model.output).is_file()
            && buffer_dir.join(format!("{output_stem}.bin")).is_file()
    }) {
        Some((models, dependencies))
    } else {
        None
    }
}

fn source_dependencies(
    models: &[ModelAsset],
    download_dir: &Path,
    materials_dir: &Path,
    includes_materials: bool,
    fingerprints: &mut FingerprintState,
) -> Result<BTreeMap<String, String>, String> {
    let mut paths = HashSet::new();
    for model in models {
        paths.extend(model_dependency_paths(
            model,
            download_dir,
            materials_dir,
            includes_materials,
        ));
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    let mut dependencies = BTreeMap::new();
    for path in paths {
        let hash = fingerprints.fingerprint(&path)?;
        dependencies.insert(cache_path(&path), hash);
    }
    Ok(dependencies)
}

fn model_dependency_paths(
    model: &ModelAsset,
    download_dir: &Path,
    materials_dir: &Path,
    includes_materials: bool,
) -> Vec<PathBuf> {
    let mut paths = HashSet::new();
    for asset_id in &model.asset_ids {
        let asset_dir = download_dir.join(asset_id);
        for filename in [
            "asset.bin",
            "asset.png",
            "asset.jpg",
            "asset.jpeg",
            "asset.webp",
            "asset.rbxm",
            "asset.rbxmx",
        ] {
            paths.insert(asset_dir.join(filename));
        }
        if let Ok(entries) = fs::read_dir(&asset_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    paths.insert(path);
                }
            }
        }
    }
    if includes_materials {
        for primitive in &model.primitives {
            paths.insert(materials_dir.join(format!("{}_color.png", primitive.material.name)));
            paths.insert(materials_dir.join(format!("{}_normal.png", primitive.material.name)));
        }
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    paths
}

struct FingerprintState {
    previous: Option<FingerprintCache>,
    current: HashMap<String, FingerprintCacheEntry>,
}

struct ModelFingerprintContext<'a> {
    download_dir: &'a Path,
    materials_dir: &'a Path,
    includes_materials: bool,
    fingerprints: &'a mut FingerprintState,
}

impl FingerprintState {
    fn load(fingerprint_path: &Path) -> Self {
        let previous = fs::read(fingerprint_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<FingerprintCache>(&bytes).ok())
            .filter(|cache| cache.version == MODEL_FORMAT_VERSION)
            .map(normalize_fingerprint_cache);
        Self {
            previous,
            current: HashMap::new(),
        }
    }

    fn fingerprint(&mut self, path: &Path) -> Result<String, String> {
        let path_string = cache_path(path);
        let metadata = match fs::metadata(path) {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => {
                return Ok(MISSING_DEPENDENCY_HASH.to_owned());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(MISSING_DEPENDENCY_HASH.to_owned());
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect dependency {}: {error}",
                    path.display()
                ));
            }
        };
        let modified_nanos = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        let hash = if let Some(entry) = self
            .previous
            .as_ref()
            .and_then(|cache| cache.files.get(&path_string))
            .filter(|entry| {
                modified_nanos.is_some()
                    && entry.length == metadata.len()
                    && entry.modified_nanos == modified_nanos
            }) {
            entry.hash.clone()
        } else {
            let bytes = fs::read(path).map_err(|error| {
                format!("failed to read dependency {}: {error}", path.display())
            })?;
            blake3::hash(&bytes).to_hex().to_string()
        };
        self.current.insert(
            path_string,
            FingerprintCacheEntry {
                length: metadata.len(),
                modified_nanos,
                hash: hash.clone(),
            },
        );
        Ok(hash)
    }

    fn write(&self, cache_path: &Path) {
        let cache = FingerprintCache {
            version: MODEL_FORMAT_VERSION,
            files: self.current.clone(),
        };
        if let Ok(bytes) = serde_json::to_vec(&cache)
            && let Err(error) = fs::write(cache_path, bytes)
        {
            eprintln!(
                "warning: failed to write dependency fingerprint cache {}: {error}",
                cache_path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn material_output_stems_are_prefixed() {
        assert_eq!(model_output_stem("hash", true), "Mhash");
        assert_eq!(model_output_stem("hash", false), "hash");
    }

    #[test]
    fn skips_model_outputs_but_keeps_mesh_stage() {
        let root = std::env::temp_dir().join(format!(
            "roform-pipeline-no-model-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let input = root.join("input.rbxmx");
        let download_dir = root.join("download");
        let mesh_payload =
            b"version 1.00\r\n1\r\n[0,0,0][0,1,0][0,0,0][1,0,0][0,1,0][1,0,0][0,0,1][0,1,0][0,1,1]\r\n";
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(download_dir.join("71515755768314")).unwrap();
        fs::write(
            &input,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/fixtures/MESH7 - hamster.rbxmx"
            )),
        )
        .unwrap();
        fs::write(
            download_dir.join("71515755768314").join("asset.bin"),
            mesh_payload,
        )
        .unwrap();

        let report = export_models(
            &input,
            &download_dir,
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                compile_models: false,
                ..ModelExportOptions::default()
            },
        )
        .unwrap();

        assert!(report.models.is_empty());
        assert!(report.failed.is_empty());
        assert!(root.join("mesh").is_dir());
        assert!(
            root.join("mesh")
                .join(format!("{}.bin", blake3::hash(mesh_payload).to_hex()))
                .is_file()
        );
        assert!(!root.join("model").exists());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reuses_a_source_independently_of_other_manifest_sources() {
        let root = std::env::temp_dir().join(format!(
            "roform-pipeline-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let model_dir = root.join("model");
        let buffer_dir = root.join("bin");
        fs::create_dir_all(&model_dir).unwrap();
        fs::create_dir_all(&buffer_dir).unwrap();
        fs::write(model_dir.join("a.gltf"), b"{}").unwrap();
        fs::write(buffer_dir.join("a.bin"), b"").unwrap();

        let manifest = ModelManifest {
            version: MODEL_FORMAT_VERSION,
            studs_per_tile: 1.0,
            includes_materials: true,
            dependencies: BTreeMap::new(),
            sources: vec![
                ModelSourceManifestEntry {
                    source: "a.rbxmx".to_owned(),
                    hash: "a".to_owned(),
                    dependencies: Vec::new(),
                },
                ModelSourceManifestEntry {
                    source: "b.rbxmx".to_owned(),
                    hash: "b".to_owned(),
                    dependencies: Vec::new(),
                },
            ],
            models: vec![ModelManifestEntry {
                hash: "a".to_owned(),
                output: model_dir.join("a.gltf").display().to_string(),
                source: "a.rbxmx".to_owned(),
                name: "A".to_owned(),
            }],
        };
        let mut fingerprints = FingerprintState {
            previous: None,
            current: HashMap::new(),
        };

        let reused =
            reusable_source_models(&manifest, "a.rbxmx", "a", &mut fingerprints, &model_dir)
                .unwrap();
        assert_eq!(reused.0.len(), 1);
        assert_eq!(reused.0[0].source, "a.rbxmx");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keeps_cached_models_in_the_manifest_for_the_next_run() {
        let root = std::env::temp_dir().join(format!(
            "roform-pipeline-manifest-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let input_dir = root.join("input");
        fs::create_dir_all(&input_dir).unwrap();
        fs::write(
            input_dir.join("model.rbxmx"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/fixtures/PRIMITIVES - hut.rbxmx"
            )),
        )
        .unwrap();
        fs::write(
            input_dir.join("model-copy.rbxmx"),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/fixtures/PRIMITIVES - hut.rbxmx"
            )),
        )
        .unwrap();

        let first = export_models(
            &input_dir,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: false,
            },
        )
        .unwrap();
        assert!(!first.models.is_empty());
        let manifest_bytes = fs::read(root.join("model").join("manifest.json")).unwrap();
        let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes).unwrap();
        assert_eq!(manifest.sources.len(), 2);
        let source_dependencies = &manifest.sources[0].dependencies;
        assert!(!source_dependencies.is_empty());
        assert!(manifest.sources.iter().all(|source| {
            source.dependencies == *source_dependencies
                && source
                    .dependencies
                    .iter()
                    .all(|path| manifest.dependencies.contains_key(path))
        }));
        assert_eq!(manifest.dependencies.len(), source_dependencies.len());

        let second = export_models(
            &input_dir,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: false,
            },
        )
        .unwrap();
        assert_eq!(second.exported, 0);
        assert_eq!(second.cached, first.models.len());

        let single_source_path = PathBuf::from(format!(
            "{}/model.rbxmx",
            input_dir.to_string_lossy().replace('\\', "/")
        ));
        let expected_single_models = first
            .models
            .iter()
            .filter(|model| model.source == cache_path(&input_dir.join("model.rbxmx")))
            .count();
        let single_source = export_models(
            &single_source_path,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: false,
            },
        )
        .unwrap();
        assert_eq!(single_source.exported, 0);
        assert_eq!(single_source.cached, expected_single_models);

        let directory_after_single_source = export_models(
            &input_dir,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: false,
            },
        )
        .unwrap();
        assert_eq!(directory_after_single_source.exported, 0);
        assert_eq!(directory_after_single_source.cached, first.models.len());

        let third = export_models(
            &input_dir,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: false,
            },
        )
        .unwrap();
        assert_eq!(third.exported, 0);
        assert_eq!(third.cached, first.models.len());

        let recompiled = export_models(
            &input_dir,
            &root.join("download"),
            &root.join("mesh"),
            &root.join("assets"),
            &root.join("model"),
            ModelExportOptions {
                studs_per_tile: 1.0,
                includes_materials: true,
                compile_models: true,
                recompile: true,
            },
        )
        .unwrap();
        assert_eq!(recompiled.exported, first.models.len());
        assert_eq!(recompiled.cached, 0);

        let first_glb = export_glbs(&recompiled.models, &root.join("glb"), false).unwrap();
        assert_eq!(
            first_glb.exported + first_glb.cached,
            recompiled.models.len()
        );
        assert!(first_glb.exported > 0);

        let cached_glb = export_glbs(&recompiled.models, &root.join("glb"), false).unwrap();
        assert_eq!(cached_glb.exported, 0);
        assert_eq!(cached_glb.cached, recompiled.models.len());

        let recompiled_glb = export_glbs(&recompiled.models, &root.join("glb"), true).unwrap();
        assert_eq!(recompiled_glb.exported, recompiled.models.len());
        assert_eq!(recompiled_glb.cached, 0);

        fs::remove_dir_all(root).unwrap();
    }
}
