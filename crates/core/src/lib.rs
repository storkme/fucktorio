//! `fucktorio_core` — shared pipeline logic for the Factorio blueprint generator.
//!
//! Used by both the PyO3 Python extension (`crates/pyo3-bindings`) and the
//! WASM web app (`crates/wasm-bindings`). Enable the `wasm` feature to gate in
//! `tsify-next`/`wasm-bindgen` derives; otherwise the crate is pure Rust.
//!
//! Pipeline: solver → bus layout → blueprint export → validation.
//! See `CLAUDE.md` and `docs/ghost-pipeline-contracts.md` for the full picture.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

pub mod analysis;
pub mod astar;
pub mod blueprint;
pub mod blueprint_parser;
pub mod bus;
pub mod common;
pub mod models;
pub mod recipe_db;
pub mod sat;
pub mod snapshot;
pub mod solver;
pub mod trace;
pub mod validate;
#[cfg(not(target_arch = "wasm32"))]
pub mod zone_cache;
