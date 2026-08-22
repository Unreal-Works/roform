#![warn(missing_docs)]

//! Convert Roblox model files into render-agnostic GLTF and GLB assets.
//!
//! The library API is centered on [`export_meshes`], [`export_models`], and
//! [`export_glbs`]. The `*_with_jobs` variants accept an explicit OS-thread
//! limit for compile work.
//! Download Roblox assets separately, for example with the `mhif` crate, and
//! pass the resulting download directory to [`export_models`]. The command
//! line binary in this package provides that download step and reports the
//! progress of each stage.

mod csg;
mod geometry;
mod gltf;
mod metadata;
mod model;
/// Model and GLB export orchestration.
pub mod pipeline;

pub use pipeline::{
    GlbReport, MeshReport, ModelExportOptions, ModelFailure, ModelManifestEntry, ModelReport,
    export_glbs, export_glbs_with_jobs, export_meshes, export_meshes_with_jobs, export_models,
    export_models_with_jobs,
};
