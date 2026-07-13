//! The nib document model, ported from the TS `frontend/src/lib/model`. Pure data +
//! geometry: everything editable normalizes to absolute cubic-bezier anchor nodes, so one
//! uniform shape covers M/L/H/V/C/S/Q/T/A on import and serializes back to a compact `d`.
//!
//! The TS unit tests are ported alongside each module as the parity oracle (`cargo test`).

pub mod document;
pub mod geometry;
pub mod path;
pub mod shapes;
pub mod types;
