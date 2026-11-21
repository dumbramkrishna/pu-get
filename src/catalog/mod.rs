//! Catalog model, loading and filtering.

pub mod filter;
pub mod loader;
pub mod model;

pub use filter::Filter;
pub use model::{
    canonical_level, BoardCatalog, BoardInfo, Catalog, CatalogEntry, Entry, FlatEntry, MaterialType,
};
