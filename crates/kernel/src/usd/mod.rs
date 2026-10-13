//! NVIDIA usd-brep (`BrepArray`) text-format (`.usda`) import and export.
//!
//! Both directions go through STEP text, as IGES does: export rewrites the
//! output of `step_export_bodies`, import restates a `BrepArray` as STEP.

mod usd_in;
mod usd_out;
mod usda;

pub use usd_in::{MAX_USD_BYTES, usd_to_step};
pub use usd_out::usd_export_bodies;

#[cfg(test)]
mod tests;
