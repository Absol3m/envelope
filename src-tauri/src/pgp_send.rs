//! Signature et chiffrement OpenPGP à l'envoi : réunit l'annuaire (clés des correspondants, clé de l'expéditeur), le
//! trousseau (clé privée), la phrase secrète et la construction PGP/MIME (`smtp::build_sealed`).
//!
//! On ne chiffre jamais pour une clé non acceptée, et jamais sans la clé de l'expéditeur : sans elle, sa propre copie
//! dans « Envoyés » serait illisible. Si une clé manque, l'envoi échoue : jamais d'envoi en clair d'un message demandé
//! chiffré ou signé.

use lettre::Message;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account};
use crate::mail::smtp::{self, OutgoingMessage, Seal};
use crate::pgp_store;

fn address(a: &str) -> Result<String> {
    let m: lettre::message::Mailbox = a.parse().map_err(|e| {
        AppError::Invalid(msg!(
            "address.invalid_named",
            "invalid address “{address}”: {detail}",
            address = a,
            detail = e
        ))
    })?;
    Ok(m.email.to_string().to_lowercase())
}

/// Construit le message signé et/ou chiffré demandé par `out`. `passphrase` : phrase de la clé de signature saisie
/// à l'instant (sinon celle que l'application a retenue, voir `pgp_store`).
pub async fn prepare(
    pool: &SqlitePool,
    account: &Account,
    out: &OutgoingMessage,
    passphrase: Option<&str>,
) -> Result<Message> {
    let from = account::sender(account, out.from_email.as_deref())?
        .email
        .to_string()
        .to_lowercase();
    let no_signing_key = || {
        AppError::Invalid(msg!(
            "pgp.no_signing_key",
            "no OpenPGP key is set for {address}: choose one in Settings",
            address = from
        ))
    };
    let own = pgp_store::signing_fingerprint(pool, &account.id, &from)
        .await?
        .ok_or_else(no_signing_key)?;

    let mut seal = Seal {
        signer: None,
        recipients: Vec::new(),
    };
    if out.pgp_sign {
        let key = pgp_store::secret_key(&own, passphrase)
            .await?
            .ok_or_else(|| {
                AppError::Invalid(msg!(
                    "pgp.no_secret_key",
                    "this key has no private part: it cannot sign"
                ))
            })?;
        seal.signer = Some((key.key, key.passphrase));
    }
    if out.pgp_encrypt {
        // Notre adresse y figure d'avance : elle est chiffrée une seule fois, avec notre propre clé (ajoutée ensuite).
        let mut seen = std::collections::HashSet::from([from.clone()]);
        for a in out.to.iter().chain(&out.cc).chain(&out.bcc) {
            let email = address(a)?;
            if !seen.insert(email.clone()) {
                continue;
            }
            let (_, key) = pgp_store::recipient_key(pool, &email)
                .await?
                .ok_or_else(|| {
                    AppError::Invalid(msg!(
                        "pgp.no_recipient_key",
                        "no accepted OpenPGP key for {address}",
                        address = email
                    ))
                })?;
            seal.recipients.push(key);
        }
        // Notre propre clé : on relit ce qu'on a envoyé.
        let key = pgp_store::public_key(pool, &own)
            .await?
            .ok_or_else(no_signing_key)?;
        seal.recipients.push(key);
    }
    let typed = passphrase.filter(|p| !p.is_empty());
    let message = smtp::build_sealed(account, out, &seal)?;
    // La phrase a servi : on la garde quelques minutes (comme au déchiffrement), pas plus.
    if let (Some(p), true) = (typed, out.pgp_sign) {
        pgp_store::remember_passphrase(&own, p);
    }
    Ok(message)
}
