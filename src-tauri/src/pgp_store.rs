//! Annuaire OpenPGP (table `pgp_keys`, clés privées dans le trousseau) et lecture d'un message PGP : c'est ici que
//! `openpgp` (cryptographie pure) rencontre la base, le trousseau et le magasin des messages bruts.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use mail_parser::MessageParser;
use pgp::composed::{Deserializable, SignedSecretKey};
use serde::Serialize as SerdeSerialize;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::{rawstore, secrets};
use crate::openpgp::{
    self, DecryptError, Imported, PgpSignature, PublicKey, SecretKey, Shape, MAX_ARMORED,
};

/// Taille maximale d'un fichier ou d'un texte de clé importé : une clé tient en quelques Kio.
const MAX_KEY: usize = 1024 * 1024;

#[derive(Debug, Clone, SerdeSerialize, sqlx::FromRow, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PgpKeyInfo {
    pub fingerprint: String,
    #[sqlx(json)]
    pub user_ids: Vec<String>,
    /// Nous avons la clé privée : on peut déchiffrer (et, plus tard, signer).
    pub has_secret: bool,
    pub revoked: bool,
}

#[derive(Debug, Clone, Default, SerdeSerialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PgpInspection {
    pub encrypted: bool,
    /// Pourquoi le contenu chiffré n'est pas affiché.
    pub error: Option<DecryptError>,
    pub signature: Option<PgpSignature>,
    /// Contenu déchiffré (ou texte d'un message signé en clair) : à afficher à la place du corps stocké.
    pub body_text: Option<String>,
    pub body_html: Option<String>,
}

const KEY_COLUMNS: &str = "fingerprint, user_ids, has_secret, revoked";

/// Durée pendant laquelle une phrase secrète saisie reste en mémoire (secondes) : comme un agent GPG, pour ne pas la redemander à chaque message.
const PASSPHRASE_TTL: i64 = 600;

fn passphrases() -> &'static Mutex<HashMap<String, (String, i64)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (String, i64)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Oublie les phrases secrètes gardées (verrouillage de l'application, suppression d'une clé).
pub fn forget_passphrases() {
    if let Ok(mut c) = passphrases().lock() {
        c.clear();
    }
}

fn cached_passphrase(fp: &str) -> Option<String> {
    let mut c = passphrases().lock().ok()?;
    match c.get(fp) {
        Some((p, until)) if *until > crate::clock::now() => Some(p.clone()),
        Some(_) => {
            c.remove(fp);
            None
        }
        None => None,
    }
}

fn remember_passphrase(fp: &str, pass: &str) {
    if let Ok(mut c) = passphrases().lock() {
        c.insert(
            fp.to_string(),
            (pass.to_string(), crate::clock::now() + PASSPHRASE_TTL),
        );
    }
}

fn invalid_key() -> AppError {
    AppError::Invalid(msg!("pgp.invalid_key", "this is not a valid OpenPGP key"))
}

pub async fn list_keys(pool: &SqlitePool) -> Result<Vec<PgpKeyInfo>> {
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {KEY_COLUMNS} FROM pgp_keys ORDER BY has_secret DESC, added_at DESC"
    )))
    .fetch_all(pool)
    .await?)
}

/// Importe une clé armurée ou binaire, publique ou privée. Une clé privée alimente aussi l'annuaire public.
pub async fn import_key(pool: &SqlitePool, data: &[u8]) -> Result<PgpKeyInfo> {
    if data.len() > MAX_KEY {
        return Err(invalid_key());
    }
    let (public, secret) = match openpgp::parse_key(data).ok_or_else(invalid_key)? {
        Imported::Public(p) => (*p, None),
        Imported::Secret(s) => (openpgp::public_of(&s), Some(s)),
    };
    let fp = public.fingerprint();
    let armored_public = openpgp::armor_public(&public).ok_or_else(invalid_key)?;
    if let Some(s) = &secret {
        let armored_secret = s
            .to_armored_string(Default::default())
            .map_err(|_| invalid_key())?;
        secrets::store_pgp_secret(&fp, armored_secret).await?;
    }
    // Importer la partie publique d'une clé dont on a déjà la privée ne retire pas la privée.
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO pgp_keys (fingerprint, armored, user_ids, has_secret, revoked, added_at) VALUES (?,?,?,?,?,?) \
         ON CONFLICT(fingerprint) DO UPDATE SET armored = excluded.armored, user_ids = excluded.user_ids, \
         revoked = excluded.revoked, has_secret = MAX(has_secret, excluded.has_secret) \
         RETURNING {KEY_COLUMNS}"
    )))
    .bind(&fp)
    .bind(&armored_public)
    .bind(serde_json::to_string(&public.user_ids())?)
    .bind(secret.is_some())
    .bind(public.revoked())
    .bind(crate::clock::now())
    .fetch_one(pool)
    .await?)
}

pub async fn delete_key(pool: &SqlitePool, fingerprint: &str) -> Result<()> {
    secrets::delete_pgp_secret(fingerprint).await?;
    sqlx::query("DELETE FROM pgp_keys WHERE fingerprint = ?")
        .bind(fingerprint)
        .execute(pool)
        .await?;
    forget_passphrases();
    Ok(())
}

async fn secret_fingerprints(pool: &SqlitePool) -> Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT fingerprint FROM pgp_keys WHERE has_secret = 1")
            .fetch_all(pool)
            .await?,
    )
}

/// Supprime toutes les clés privées (réinitialisation de l'application) : trousseau et annuaire.
pub async fn delete_all_secrets(pool: &SqlitePool) -> Result<()> {
    for fp in secret_fingerprints(pool).await? {
        secrets::delete_pgp_secret(&fp).await?;
    }
    sqlx::query("DELETE FROM pgp_keys WHERE has_secret = 1")
        .execute(pool)
        .await?;
    forget_passphrases();
    Ok(())
}

async fn public_keys(pool: &SqlitePool) -> Result<Vec<PublicKey>> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT armored FROM pgp_keys WHERE revoked = 0")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .filter_map(|a| openpgp::parse_public(a))
        .collect())
}

/// Clés privées, avec la phrase secrète connue (saisie à l'instant, sinon gardée en mémoire). Les lectures du
/// trousseau sont les plus lentes : on ne les fait que pour un message réellement chiffré.
async fn secret_keys(pool: &SqlitePool, typed: Option<&str>) -> Result<Vec<SecretKey>> {
    let mut out = Vec::new();
    for fingerprint in secret_fingerprints(pool).await? {
        let Some(armored) = secrets::load_pgp_secret(&fingerprint).await? else {
            continue;
        };
        if armored.len() > MAX_ARMORED {
            continue;
        }
        if let Ok((key, _)) = SignedSecretKey::from_string(&armored) {
            let passphrase = typed
                .map(str::to_string)
                .or_else(|| cached_passphrase(&fingerprint));
            out.push(SecretKey {
                key,
                fingerprint,
                passphrase,
            });
        }
    }
    Ok(out)
}

fn text_of(data: &[u8], mime: bool) -> (Option<String>, Option<String>) {
    if mime {
        if let Some(m) = MessageParser::default().parse(data) {
            return (
                m.body_text(0).map(|c| c.into_owned()),
                m.body_html(0).map(|c| c.into_owned()),
            );
        }
    }
    (Some(String::from_utf8_lossy(data).into_owned()), None)
}

/// Ce que l'analyse du message brut (calcul) laisse à faire : rien, un verdict, ou un déchiffrement.
enum Step {
    NotPgp,
    Done(PgpInspection),
    Decrypt { armored: Vec<u8>, mime: bool },
}

fn analyse(raw: &[u8], publics: &[PublicKey]) -> Step {
    let Some(shape) = openpgp::shape(raw) else {
        return Step::NotPgp;
    };
    Step::Done(match shape {
        Shape::Signed { content, signature } => PgpInspection {
            signature: openpgp::verify_detached(&signature, &content, publics),
            ..Default::default()
        },
        Shape::Cleartext { armored } => {
            let (body_text, signature) = openpgp::verify_cleartext(&armored, publics).unzip();
            PgpInspection {
                signature,
                body_text,
                ..Default::default()
            }
        }
        Shape::Encrypted { armored, mime } => return Step::Decrypt { armored, mime },
    })
}

/// Déchiffre et en tire l'affichage ; rend aussi l'empreinte de la clé qui a servi.
fn open_encrypted(
    armored: &[u8],
    mime: bool,
    secrets: &[SecretKey],
    publics: &[PublicKey],
) -> (PgpInspection, Option<String>) {
    match openpgp::decrypt(armored, secrets, publics) {
        Ok(d) => {
            let (body_text, body_html) = text_of(&d.data, mime);
            // Message signé puis chiffré : la signature est dans le contenu déchiffré (OpenPGP) ou dans l'entité MIME.
            let signature = d.signature.or_else(|| match openpgp::shape(&d.data) {
                Some(Shape::Signed { content, signature }) if mime => {
                    openpgp::verify_detached(&signature, &content, publics)
                }
                _ => None,
            });
            (
                PgpInspection {
                    encrypted: true,
                    signature,
                    body_text,
                    body_html,
                    ..Default::default()
                },
                Some(d.fingerprint),
            )
        }
        Err(e) => (
            PgpInspection {
                encrypted: true,
                error: Some(e),
                ..Default::default()
            },
            None,
        ),
    }
}

fn interrupted(e: tokio::task::JoinError) -> AppError {
    AppError::Invalid(msg!(
        "pgp.interrupted",
        "decryption interrupted: {detail}",
        detail = e
    ))
}

/// Lit la forme PGP d'un message : signature, ou déchiffrement. `None` : le message n'est pas PGP. Tout le calcul
/// (analyse, vérifications, déchiffrement) se fait hors de l'exécuteur.
pub async fn inspect(
    pool: &SqlitePool,
    raw_dir: &Path,
    message_id: i64,
    passphrase: Option<&str>,
) -> Result<Option<PgpInspection>> {
    let hash: Option<String> = sqlx::query_scalar("SELECT raw_hash FROM messages WHERE id = ?")
        .bind(message_id)
        .fetch_optional(pool)
        .await?
        .flatten();
    let Some(hash) = rawstore::usable(hash.as_deref()) else {
        return Ok(None);
    };
    let raw = rawstore::get(raw_dir, hash).await?;
    let publics = Arc::new(public_keys(pool).await?);
    let for_task = publics.clone();
    let step = tokio::task::spawn_blocking(move || analyse(&raw, &for_task))
        .await
        .map_err(interrupted)?;
    let (armored, mime) = match step {
        Step::NotPgp => return Ok(None),
        Step::Done(r) => return Ok(Some(r)),
        Step::Decrypt { armored, mime } => (armored, mime),
    };
    let keys = secret_keys(pool, passphrase).await?;
    let (result, opened_with) =
        tokio::task::spawn_blocking(move || open_encrypted(&armored, mime, &keys, &publics))
            .await
            .map_err(interrupted)?;
    // Seule la clé qui a déchiffré garde la phrase saisie.
    if let (Some(fp), Some(p)) = (opened_with, passphrase) {
        remember_passphrase(&fp, p);
    }
    Ok(Some(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpgp::public_of;
    use crate::openpgp::tests::{keygen, mime_signed, sign, PART};
    use crate::openpgp::SigState;

    async fn message_with(pool: &SqlitePool, dir: &Path, raw: &[u8]) -> i64 {
        let hash = rawstore::put(dir, raw).await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','a','a','i','s')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO folders (id,account_id,path,name,role) VALUES (900,'a','INBOX','INBOX','inbox')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO messages (id,account_id,folder_id,uid,from_addr,date,raw_hash) VALUES (1,'a',900,1,'alice@x.fr','2026-01-01T00:00:00Z',?)")
            .bind(hash)
            .execute(pool)
            .await
            .unwrap();
        1
    }

    #[tokio::test]
    async fn signature_verifiee_apres_import_de_la_cle_publique() {
        let pool = crate::db::init_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-pgp-{}", uuid::Uuid::new_v4()));
        let key = keygen("Alice <alice@x.fr>", None);
        let id = message_with(
            &pool,
            &dir,
            &mime_signed(PART, &sign(&key, PART.as_bytes())),
        )
        .await;

        // Sans la clé : on ne peut pas conclure.
        let r = inspect(&pool, &dir, id, None).await.unwrap().unwrap();
        assert_eq!(r.signature.unwrap().state, SigState::UnknownKey);

        let armored = openpgp::armor_public(&public_of(&key)).unwrap();
        let info = import_key(&pool, armored.as_bytes()).await.unwrap();
        assert_eq!(info.user_ids, vec!["Alice <alice@x.fr>"]);
        assert!(!info.has_secret);
        // Réimporter ne duplique pas.
        import_key(&pool, armored.as_bytes()).await.unwrap();
        assert_eq!(list_keys(&pool).await.unwrap().len(), 1);

        let r = inspect(&pool, &dir, id, None).await.unwrap().unwrap();
        let s = r.signature.unwrap();
        assert_eq!(
            (s.state, s.signer.as_deref()),
            (SigState::Valid, Some("Alice <alice@x.fr>"))
        );
        assert!(!r.encrypted);

        // Une clé révoquée ne sert plus à vérifier.
        sqlx::query("UPDATE pgp_keys SET revoked = 1")
            .execute(&pool)
            .await
            .unwrap();
        let r = inspect(&pool, &dir, id, None).await.unwrap().unwrap();
        assert_eq!(r.signature.unwrap().state, SigState::UnknownKey);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn message_sans_pgp_et_cle_invalide() {
        let pool = crate::db::init_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-pgp-{}", uuid::Uuid::new_v4()));
        let id = message_with(&pool, &dir, b"Subject: s\r\n\r\nbonjour").await;
        assert!(inspect(&pool, &dir, id, None).await.unwrap().is_none());
        assert!(import_key(&pool, b"pas une cl\xc3\xa9").await.is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
