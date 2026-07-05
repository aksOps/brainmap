//! Embedded model data chunk for brainmap-cli.
//!
//! This crate is an implementation detail that keeps each crates.io package
//! under the registry upload size cap.

pub const BYTES: &[u8] = include_bytes!("../data/part.bin");
