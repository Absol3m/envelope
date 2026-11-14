//! Secrets dans le trousseau du système : mots de passe et jetons OAuth2 des comptes (service `dev.envelope.app`, clés
//! `<account_id>:password` / `<account_id>:oauth`), identifiants d'application OAuth2 et secret de l'application
//! elle-même. Jamais en base ni dans les journaux. `account` ne touche plus qu'à la base : c'est ici qu'un secret
//! naît, se relit et se supprime.

use envelope_core::kinds::AuthKind;

use crate::error::{AppError, Result};
use crate::mail::oauth::{self, OAuthProvider};

const KEYRING_SERVICE: &str = "dev.envelope.app";
const PASSWORD: &str = "password";
const OAUTH: &str = "oauth";

/// Secret d'un compte, selon son mode d'authentification.
pub enum Secret {
    Password(String),
    OAuth(oauth::OAuthTokens),
}

impl Secret {
    pub fn auth_kind(&self) -> AuthKind {
        match self {
            Secret::Password(_) => AuthKind::Password,
            Secret::OAuth(_) => AuthKind::OAuth2,
        }
    }
}

/// Enregistre le secret d'un compte (accès au trousseau hors de l'exécuteur).
pub async fn store(account_id: &str, secret: Secret) -> Result<()> {
    let id = account_id.to_string();
    blocking_keychain(move || match secret {
        Secret::Password(p) => store_password(&id, &p),
        Secret::OAuth(t) => store_oauth(&id, &t),
    })
    .await
}

/// Mot de passe d'un compte.
pub async fn password(account_id: &str) -> Result<String> {
    let id = account_id.to_string();
    blocking_keychain(move || load_password(&id)).await
}

/// Jetons OAuth2 d'un compte.
pub async fn oauth_tokens(account_id: &str) -> Result<oauth::OAuthTokens> {
    // Cache d'abord, sans quitter l'exécuteur : chaque connexion IMAP/SMTP passe ici.
    if let Some(t) = cached_tokens(account_id) {
        return Ok(t);
    }
    let id = account_id.to_string();
    blocking_keychain(move || load_oauth(&id)).await
}

/// Supprime tous les secrets d'un compte.
pub async fn delete(account_id: &str) -> Result<()> {
    let id = account_id.to_string();
    blocking_keychain(move || {
        delete_secrets(&id);
        Ok(())
    })
    .await
}

fn entry(account_id: &str, kind: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new(
        KEYRING_SERVICE,
        &format!("{account_id}:{kind}"),
    )?)
}
fn store_password(account_id: &str, password: &str) -> Result<()> {
    Ok(entry(account_id, PASSWORD)?.set_password(password)?)
}
fn load_password(account_id: &str) -> Result<String> {
    Ok(entry(account_id, PASSWORD)?.get_password()?)
}
/// Jetons OAuth déjà lus ou écrits, gardés en mémoire : chaque connexion IMAP/SMTP en demande un, et une lecture du
/// trousseau coûte (voire une autorisation) alors que le jeton ne change qu'au rafraîchissement.
/// Le trousseau reste la référence : `store_oauth` et `delete_secrets` tiennent le cache à jour.
fn token_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, oauth::OAuthTokens>>
{
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, oauth::OAuthTokens>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}
fn store_oauth(account_id: &str, tokens: &oauth::OAuthTokens) -> Result<()> {
    entry(account_id, OAUTH)?.set_password(&serde_json::to_string(tokens)?)?;
    if let Ok(mut cache) = token_cache().lock() {
        cache.insert(account_id.to_string(), tokens.clone());
    }
    Ok(())
}
fn cached_tokens(account_id: &str) -> Option<oauth::OAuthTokens> {
    token_cache().lock().ok()?.get(account_id).cloned()
}
fn load_oauth(account_id: &str) -> Result<oauth::OAuthTokens> {
    if let Some(t) = cached_tokens(account_id) {
        return Ok(t);
    }
    let tokens: oauth::OAuthTokens =
        serde_json::from_str(&entry(account_id, OAUTH)?.get_password()?)?;
    if let Ok(mut cache) = token_cache().lock() {
        cache.insert(account_id.to_string(), tokens.clone());
    }
    Ok(tokens)
}
/// Secret de l'application elle-même (hors compte), sous l'usage `kind` : le verrouillage par mot de passe.
fn app_entry(kind: &str) -> Result<keyring::Entry> {
    entry("app", kind)
}
/// Range un secret de l'application ; `make` le calcule dans le même saut hors de l'exécuteur que l'écriture (un
/// hachage lent, par exemple).
pub async fn store_app_secret(
    kind: &'static str,
    make: impl FnOnce() -> Result<String> + Send + 'static,
) -> Result<()> {
    blocking_keychain(move || Ok(app_entry(kind)?.set_password(&make()?)?)).await
}
/// Lecture synchrone : appelée depuis un calcul déjà hors de l'exécuteur (vérification du mot de passe).
pub fn load_app_secret(kind: &str) -> Result<Option<String>> {
    match app_entry(kind)?.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub async fn delete_app_secret(kind: &'static str) -> Result<()> {
    blocking_keychain(move || {
        if let Ok(e) = app_entry(kind) {
            let _ = e.delete_credential();
        }
        Ok(())
    })
    .await
}

/// Identifiants d'application OAuth2 (par fournisseur, partagés par tous les comptes).
fn client_entry(provider: OAuthProvider) -> Result<keyring::Entry> {
    entry("oauth-client", provider.provider().as_str())
}
pub async fn store_client(provider: OAuthProvider, creds: oauth::ClientCreds) -> Result<()> {
    blocking_keychain(move || {
        Ok(client_entry(provider)?.set_password(&serde_json::to_string(&creds)?)?)
    })
    .await
}
/// Lecture synchrone : `OAuthProvider::client` (trousseau puis variables d'environnement) l'appelle dans un saut
/// hors de l'exécuteur.
pub fn load_client(provider: OAuthProvider) -> Result<Option<oauth::ClientCreds>> {
    match client_entry(provider)?.get_password() {
        Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub async fn delete_client(provider: OAuthProvider) -> Result<()> {
    blocking_keychain(move || {
        if let Ok(e) = client_entry(provider) {
            let _ = e.delete_credential();
        }
        Ok(())
    })
    .await
}
fn delete_secrets(account_id: &str) {
    if let Ok(mut cache) = token_cache().lock() {
        cache.remove(account_id);
    }
    for kind in [PASSWORD, OAUTH] {
        if let Ok(e) = entry(account_id, kind) {
            let _ = e.delete_credential();
        }
    }
}

/// Accès au trousseau du système depuis du code asynchrone : l'appel est bloquant (jusqu'à plusieurs secondes si le
/// trousseau est verrouillé ou demande une autorisation) et ne doit pas occuper un thread de l'exécuteur.
pub(crate) async fn blocking_keychain<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(job).await.map_err(|e| {
        AppError::Auth(msg!(
            "keychain.interrupted",
            "keychain access interrupted: {detail}",
            detail = e
        ))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un accès bloquant au trousseau ne doit pas figer l'exécuteur : sur un exécuteur à UN seul fil, une tâche
    /// concurrente continue d'avancer pendant que « le trousseau » dort.
    #[tokio::test(flavor = "current_thread")]
    async fn acces_trousseau_ne_bloque_pas_l_executeur() {
        let ticks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = ticks.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        });
        let value = blocking_keychain(|| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(42)
        })
        .await
        .unwrap();
        ticker.abort();
        assert_eq!(value, 42);
        assert!(
            ticks.load(std::sync::atomic::Ordering::SeqCst) >= 10,
            "l'exécuteur a continué à tourner : {} battements",
            ticks.load(std::sync::atomic::Ordering::SeqCst)
        );
        // Une erreur du trousseau remonte telle quelle.
        let err = blocking_keychain::<()>(|| Err(AppError::Auth("refusé".into())))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refusé"));
    }
}
