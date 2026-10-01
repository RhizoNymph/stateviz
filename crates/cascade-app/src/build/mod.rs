//! Build mode's pure logic: edit ops from toolbar actions, gestures and
//! inspector fields; the commit pipeline; undo/redo; the file on disk.
//!
//! GPUI glue lives in `workspace::building` and `panels::build`.

pub mod connect;
pub mod defs;
pub mod disk;
pub mod inspector;
pub mod ops;
pub mod pipeline;
pub mod undo;
