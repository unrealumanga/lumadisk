//! Thin library facade so benches/examples can call the real scanner code.
//! The application binary (`main.rs`) declares its own modules and is unaffected.
pub mod model;
pub mod scanner;
pub mod treemap;
