//! oabctl library — OAB agent provisioning primitives for ECS.
//!
//! The `oabctl` binary is a thin CLI shell over this library. Downstream
//! services (e.g. control planes that provision OAB agents programmatically)
//! can depend on this crate to reuse manifest validation, ECS service
//! reconciliation, ingress (Cloud Map + VPC Link + API Gateway) setup, and
//! safe delete primitives in-process instead of shelling out to the CLI.
//!
//! Key entry points:
//! - [`manifest::OABServiceManifest`] — typed manifest (serde), with
//!   [`manifest::OABServiceManifest::validate`]
//! - [`apply::apply_manifests`] — validate + reconcile ECS services and
//!   ingress from in-memory manifests
//! - [`apply::run`] — file-path variant used by the CLI
//! - [`delete::run`] / [`delete::run_from_file`] — teardown
//! - [`scale::run`] — desired-count scaling

pub mod apply;
pub mod bootstrap;
pub mod config;
pub mod create;
pub mod delete;
pub mod get;
pub mod ingress;
pub mod manifest;
pub mod scale;
pub mod secrets;
