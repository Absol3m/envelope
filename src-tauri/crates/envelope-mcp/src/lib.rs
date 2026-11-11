//! Serveur MCP d'Envelope (temps 1 : lecture seule), sur le SDK officiel Rust (`rmcp`).
//!
//! - [`tools`] : les outils d'Envelope, branchés sur `envelope-core` ; le SDK fournit le protocole
//!   (JSON-RPC, cycle de vie, négociation de version, validation des paramètres) ;
//! - [`db`]    : ouverture de la base de l'application en lecture seule.

pub mod db;
pub mod tools;
