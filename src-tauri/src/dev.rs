//! Outils de développement, absents des builds de production (`debug_assertions`).
//!
//! `seed_inbox` dépose des messages d'essai dans la boîte de réception d'un compte déjà configuré
//! dans l'application, directement par IMAP (APPEND) : pas d'envoi SMTP, donc pas de SPF/DMARC.
//! Les expéditeurs sont fictifs, en `example.com` (domaine réservé : aucune usurpation).

use std::path::Path;
use std::time::{Duration, SystemTime};

use base64::Engine;
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::Message;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::{account, imap};

/// 1 pixel PNG (base64) : pièce jointe minuscule mais réelle.
const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

struct Seed {
    id: &'static str,
    name: &'static str,
    addr: &'static str,
    subject: &'static str,
    body: &'static str,
    hours_ago: u64,
    reply_to: Option<&'static str>,
    attachment: Option<&'static str>,
    unread: bool,
    /// En-têtes List-Unsubscribe à ajouter (liste de diffusion).
    unsub: Option<&'static str>,
}

const SEEDS: &[Seed] = &[
    Seed { id: "seed-01", name: "Camille Laurent", addr: "camille.laurent@example.com", subject: "Compte rendu : atelier design du 3 novembre", body: "Bonjour,\n\nVoici le compte rendu de l'atelier :\n- Palette validée\n- Typographie à revoir\n- Prochaine session le 17\n\nCamille", hours_ago: 30, reply_to: None, attachment: None, unread: true, unsub: None },
    Seed { id: "seed-02", name: "Hugo Martin", addr: "hugo.martin@example.com", subject: "Devis hébergement envelope.app", body: "Bonjour,\n\nPouvez-vous me confirmer le devis pour l'hébergement de la version 3 comptes ?\n\nMerci,\nHugo", hours_ago: 28, reply_to: None, attachment: None, unread: false, unsub: None },
    Seed { id: "seed-03", name: "Hugo Martin", addr: "hugo.martin@example.com", subject: "Re: Devis hébergement envelope.app", body: "Merci pour le retour, je valide la version à 3 comptes mail. On part là-dessus.\n\nHugo", hours_ago: 26, reply_to: Some("seed-02"), attachment: None, unread: true, unsub: None },
    Seed { id: "seed-04", name: "Service facturation", addr: "facturation@example.com", subject: "Votre facture de novembre est disponible", body: "Bonjour,\n\nVotre facture de novembre (12,00 EUR) est disponible dans votre espace client.\n\nLe service facturation", hours_ago: 24, reply_to: None, attachment: None, unread: true, unsub: None },
    Seed { id: "seed-05", name: "Léa Fontaine", addr: "lea.fontaine@example.com", subject: "Photos du week-end", body: "Coucou ! Je te joins une première photo, les autres suivent dès que j'ai du réseau.\n\nLéa", hours_ago: 20, reply_to: None, attachment: Some("photo-weekend.png"), unread: true, unsub: None },
    Seed { id: "seed-06", name: "Boutique Soldes", addr: "promo@example.com", subject: "Dernières heures : -60 % sur tout le site", body: "Profitez-en avant minuit ! Livraison offerte dès 49 EUR.\n\nPour vous désabonner, répondez STOP.", hours_ago: 14, reply_to: None, attachment: None, unread: true, unsub: Some("List-Unsubscribe: <mailto:desabo@example.com?subject=stop>, <https://example.com/desabo/promo>\r\n") },
    Seed { id: "seed-07", name: "Équipe Projet", addr: "projet@example.com", subject: "[envelope] Pull request fusionnée : multiplateforme", body: "La pull request « Rendu natif sur Windows, macOS et Linux » a été fusionnée dans main.", hours_ago: 10, reply_to: None, attachment: None, unread: false, unsub: None },
    Seed { id: "seed-08", name: "Marc Dupuis", addr: "marc.dupuis@example.com", subject: "Invitation : point d'équipe jeudi 10h", body: "Bonjour à tous,\n\nPoint d'équipe jeudi à 10h en salle Bleue. Merci de confirmer votre présence.\n\nMarc", hours_ago: 6, reply_to: None, attachment: None, unread: true, unsub: None },
    Seed { id: "seed-09", name: "Newsletter Tech", addr: "newsletter@example.com", subject: "Cette semaine : Rust, Tauri 2 et le futur du mail", body: "Au sommaire :\n1. Les nouveautés de Rust\n2. Tauri 2 en production\n3. Le courrier électronique en 2026\n\nBonne lecture !", hours_ago: 4, reply_to: None, attachment: None, unread: true, unsub: Some("List-Unsubscribe: <https://example.com/desabo/news>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n") },
    Seed { id: "seed-10", name: "Sophie Bernard", addr: "sophie.bernard@example.com", subject: "Question sur le contrat de maintenance", body: "Bonjour,\n\nPourriez-vous m'indiquer la durée d'engagement du contrat de maintenance ?\n\nCordialement,\nSophie Bernard", hours_ago: 2, reply_to: None, attachment: None, unread: true, unsub: None },
];

fn invalid(e: impl std::fmt::Display) -> AppError {
    AppError::Invalid(e.to_string())
}

fn build(seed: &Seed, to: &Mailbox) -> Result<Vec<u8>> {
    let from: Mailbox = format!("{} <{}>", seed.name, seed.addr)
        .parse()
        .map_err(invalid)?;
    let mut b = Message::builder()
        .from(from)
        .to(to.clone())
        .subject(seed.subject)
        .date(SystemTime::now() - Duration::from_secs(seed.hours_ago * 3600))
        .message_id(Some(format!("<{}@example.com>", seed.id)));
    if let Some(parent) = seed.reply_to {
        let parent = format!("<{parent}@example.com>");
        b = b.in_reply_to(parent.clone()).references(parent);
    }
    let text = SinglePart::plain(seed.body.to_string());
    let msg = match seed.attachment {
        None => b.singlepart(text),
        Some(name) => b.multipart(
            MultiPart::mixed().singlepart(text).singlepart(
                Attachment::new(name.to_string()).body(
                    base64::engine::general_purpose::STANDARD
                        .decode(PNG_BASE64)
                        .map_err(invalid)?,
                    ContentType::parse("image/png").map_err(invalid)?,
                ),
            ),
        ),
    };
    let raw = msg.map_err(invalid)?.formatted();
    // Les en-têtes de liste vont en tête : l'ordre des en-têtes n'a pas d'importance (RFC 5322).
    Ok([seed.unsub.unwrap_or_default().as_bytes(), &raw].concat())
}

/// Dépose les messages d'essai dans la boîte de réception du compte `email` (déjà configuré).
/// Les identifiants viennent du trousseau, comme pour l'application. Retourne le nombre déposé.
pub async fn seed_inbox(db_path: &Path, email: &str) -> Result<usize> {
    // Même fournisseur crypto que l'application (un seul par process).
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pool = SqlitePool::connect_with(
        SqliteConnectOptions::new()
            .filename(db_path)
            .read_only(true),
    )
    .await?;
    let id: String = sqlx::query_scalar("SELECT id FROM accounts WHERE lower(email) = lower(?)")
        .bind(email)
        .fetch_optional(&pool)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("compte {email}")))?;
    let acct = account::get(&pool, &id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let mut session = imap::connect(&acct, &cred).await?;
    let inbox = imap::find_or_create(&mut session, "inbox", "INBOX").await?;
    let to: Mailbox = acct.email.parse().map_err(invalid)?;
    for seed in SEEDS {
        let raw = build(seed, &to)?;
        let flags = (!seed.unread).then_some("(\\Seen)");
        session.append(&inbox, flags, None, &raw).await?;
    }
    let _ = session.logout().await;
    Ok(SEEDS.len())
}
