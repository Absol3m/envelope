//! Configuration TLS commune (IMAP, SMTP, appels HTTPS) : **le système vérifie les certificats**.
//!
//! `rustls-platform-verifier` délègue à l'API native de chaque OS (Security.framework sur macOS,
//! CryptoAPI sur Windows, magasin de certificats du système sous Linux). Les autorités ajoutées
//! par l'utilisateur ou l'entreprise sont donc prises en compte, la révocation et les règles du
//! système aussi, et on n'embarque ni ne maintient aucune liste d'autorités.

use std::sync::{Arc, OnceLock};

use rustls_platform_verifier::BuilderVerifierExt;
use tokio_rustls::rustls::ClientConfig;

use crate::error::{AppError, Result};

/// Configuration TLS client, construite une fois.
pub fn client_config() -> Result<Arc<ClientConfig>> {
    static CFG: OnceLock<Arc<ClientConfig>> = OnceLock::new();
    if let Some(cfg) = CFG.get() {
        return Ok(cfg.clone());
    }
    // Un seul fournisseur cryptographique par processus ; sans effet s'il est déjà installé.
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let cfg = ClientConfig::builder()
        .with_platform_verifier()
        .map_err(|e| AppError::Invalid(format!("vérification des certificats indisponible : {e}")))?
        .with_no_client_auth();
    Ok(CFG.get_or_init(|| Arc::new(cfg)).clone())
}

/// Client HTTP soumis aux mêmes règles de vérification (jamais de racines embarquées).
pub fn http_builder() -> Result<reqwest::ClientBuilder> {
    Ok(reqwest::Client::builder().use_preconfigured_tls(client_config()?.as_ref().clone()))
}
