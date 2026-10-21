//! Shop products for the hosted service: a product link becomes an
//! identity ([`identity`]), its data comes from `AnySite` ([`anysite`],
//! [`import`]) and is read by fixed rules ([`extract`], [`page`]) into
//! fields that each say where they came from ([`product`]). What a
//! project costs is the takeoff joined with those fields ([`estimate`]),
//! in exact arithmetic ([`exact`]).
//!
//! No language model reads anything here: what the rules cannot read is
//! returned as missing, and the calling agent fills it.

pub mod anysite;
pub mod decimal;
pub mod estimate;
pub mod exact;
pub mod extract;
pub mod identity;
pub mod import;
pub mod page;
pub mod product;

pub use anysite::AnySite;
pub use identity::{Identity, identify};
pub use product::{Evidence, Field, Fields, Kind};
