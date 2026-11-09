//! Serveur MCP d'Envelope (temps 1 : lecture seule).
//!
//! - [`protocol`] : JSON-RPC 2.0 sur stdio + cycle de vie MCP, sans aucune connaissance du mail ;
//! - [`schema`]   : définitions d'outils (annotations d'effet) et validation des arguments ;
//! - [`tools`]    : les outils d'Envelope, branchés sur `envelope-core` ;
//! - [`db`]       : ouverture de la base de l'application en lecture seule.
//!
//! L'architecture (trait `Tools` + serveur générique, annotations d'effet, validation des
//! arguments avant exécution) s'inspire du serveur MCP de SolidCraft (MIT OR Apache-2.0).

pub mod db;
pub mod protocol;
pub mod schema;
pub mod tools;
