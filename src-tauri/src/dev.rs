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
use envelope_core::role::FolderRole;

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
    AppError::Invalid((e.to_string()).into())
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
        .ok_or_else(|| AppError::NotFound((format!("compte {email}")).into()))?;
    let acct = account::get(&pool, &id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let mut session = imap::connect(&acct, &cred).await?;
    let inbox = imap::find_or_create(&mut session, FolderRole::Inbox, "INBOX").await?;
    let to: Mailbox = acct.email.parse().map_err(invalid)?;
    for seed in SEEDS {
        let raw = build(seed, &to)?;
        let flags = (!seed.unread).then_some("(\\Seen)");
        session.append(&inbox, flags, None, &raw).await?;
    }
    let _ = session.logout().await;
    Ok(SEEDS.len())
}

// ───────────── Base de test volumineuse ─────────────

/// Générateur pseudo-aléatoire déterministe (xorshift) : mêmes données à chaque exécution.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: &[&str] = &[
    "budget",
    "projet",
    "réunion",
    "client",
    "facture",
    "contrat",
    "livraison",
    "équipe",
    "planning",
    "version",
    "serveur",
    "mise",
    "production",
    "rapport",
    "demande",
    "validation",
    "retour",
    "devis",
    "atelier",
    "migration",
    "sécurité",
    "formation",
    "support",
    "incident",
    "maintenance",
    "commande",
    "paiement",
    "agenda",
    "objectif",
    "résultat",
    "analyse",
    "proposition",
    "document",
    "partage",
    "accès",
    "compte",
    "mot",
    "passe",
    "installation",
    "mise à jour",
    "tarif",
    "abonnement",
];
const FIRST: &[&str] = &[
    "Alice", "Bruno", "Camille", "David", "Emma", "Farid", "Gaëlle", "Hugo", "Inès", "Julien",
    "Karim", "Léa", "Marc", "Nina", "Omar", "Paul",
];
const LAST: &[&str] = &[
    "Martin", "Bernard", "Dubois", "Thomas", "Robert", "Petit", "Durand", "Leroy", "Moreau",
    "Simon", "Laurent", "Lefebvre",
];

/// Phrase de `min` à `min + spread - 1` mots.
fn sentence(rng: &mut Rng, min: usize, spread: usize) -> String {
    let words = min + rng.below(spread.max(1));
    let s: Vec<&str> = (0..words).map(|_| WORDS[rng.below(WORDS.len())]).collect();
    let out = s.join(" ") + ".";
    let mut chars = out.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

/// Ouvre (et migre) la base de test `path`.
pub async fn open_pool(path: &Path) -> Result<SqlitePool> {
    crate::db::init(path).await
}

/// Remplit la base `db_path` de `count` messages réalistes (corps de 1 à 6 Ko, 40 % en HTML de
/// 5 à 30 Ko, 30 % en réponses dans des conversations, dates sur 3 ans) pour mesurer la mémoire et
/// les temps de requête. Crée un compte fictif `bench@example.com` ; rien n'est envoyé ni lu en IMAP.
pub async fn seed_local(db_path: &Path, count: usize) -> Result<()> {
    use crate::mail::ingest::{self, Flags, MessageLoc, ParsedMessage};
    use chrono::{Duration as Dur, SecondsFormat, Utc};
    use envelope_core::addr::{self, Addr};
    use std::collections::HashSet;

    let pool = crate::db::init(db_path).await?;
    sqlx::query("INSERT OR IGNORE INTO accounts (id, email, username, imap_host, smtp_host) VALUES ('bench','bench@example.com','bench','invalid.invalid','invalid.invalid')")
        .execute(&pool).await?;
    let mut folders = Vec::new();
    for (path, role) in [
        ("INBOX", "inbox"),
        ("Sent", "sent"),
        ("Archive", "archive"),
        ("Trash", "trash"),
    ] {
        sqlx::query("INSERT OR IGNORE INTO folders (account_id, path, name, role) VALUES ('bench', ?1, ?1, ?2)")
            .bind(path).bind(role).execute(&pool).await?;
        folders.push(
            sqlx::query_scalar::<_, i64>(
                "SELECT id FROM folders WHERE account_id = 'bench' AND path = ?",
            )
            .bind(path)
            .fetch_one(&pool)
            .await?,
        );
    }
    let dir = std::env::temp_dir().join("envelope-bench-attachments");
    let me = Addr {
        name: Some("Bench".into()),
        address: "bench@example.com".into(),
    };
    let now = Utc::now();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut touched = HashSet::new();
    let mut last_ids: Vec<(String, String)> = Vec::new(); // (message-id, sujet) récents pour les réponses
    let mut uids = [0i64; 4];

    for i in 0..count {
        let sender = format!(
            "{} {}",
            FIRST[rng.below(FIRST.len())],
            LAST[rng.below(LAST.len())]
        );
        let addr = format!(
            "{}.{}@example.com",
            sender.split(' ').next().unwrap().to_lowercase(),
            sender.split(' ').nth(1).unwrap().to_lowercase()
        );
        let reply = !last_ids.is_empty() && rng.below(100) < 30;
        let (subject, parent) = if reply {
            let (id, subj) = last_ids[rng.below(last_ids.len())].clone();
            (format!("Re: {}", subj.trim_start_matches("Re: ")), Some(id))
        } else {
            (sentence(&mut rng, 3, 4), None)
        };
        let message_id = format!("bench-{i}@example.com");
        let paragraphs = 1 + rng.below(5);
        let text: String = (0..paragraphs)
            .map(|_| sentence(&mut rng, 20, 60))
            .collect::<Vec<_>>()
            .join("\n\n");
        let html = (rng.below(100) < 40).then(|| {
            let blocks = 8 + rng.below(40);
            let body: String = (0..blocks)
                .map(|_| format!("<tr><td style=\"padding:8px 16px;font:14px Arial,sans-serif;color:#333\"><p>{}</p><a href=\"https://example.com/l/{}\">Lire la suite</a></td></tr>", sentence(&mut rng, 25, 40), rng.below(10_000)))
                .collect();
            format!("<html><body><table width=\"600\" style=\"margin:0 auto\">{body}</table></body></html>")
        });
        let ts = now - Dur::minutes(rng.below(3 * 365 * 24 * 60) as i64);
        let slot = match rng.below(100) {
            0..=69 => 0,
            70..=79 => 1,
            80..=94 => 2,
            _ => 3,
        };
        uids[slot] += 1;
        let from_me = slot == 1;
        let parsed = ParsedMessage {
            message_id: Some(message_id.clone()),
            in_reply_to: parent.clone(),
            references: parent.into_iter().collect(),
            subject: subject.clone(),
            from_name: Some(if from_me {
                "Bench".into()
            } else {
                sender.clone()
            }),
            from_addr: if from_me {
                me.address.clone()
            } else {
                addr.clone()
            },
            to: addr::encode(&[if from_me {
                Addr {
                    name: Some(sender),
                    address: addr,
                }
            } else {
                me.clone()
            }]),
            cc: "[]".into(),
            bcc: "[]".into(),
            date: ts.to_rfc3339_opts(SecondsFormat::Secs, true),
            snippet: text.chars().take(200).collect(),
            body_text: Some(text),
            body_html: html,
            attachments: vec![],
            unsubscribe: None,
            draft_id: None,
            auth_fail: None,
        };
        let tid = ingest::store(
            &pool,
            &dir,
            MessageLoc {
                account_id: "bench",
                folder_id: folders[slot],
                uid: uids[slot],
                size: None,
            },
            Flags {
                seen: rng.below(100) >= 20,
                flagged: rng.below(100) < 3,
            },
            parsed,
        )
        .await?;
        touched.insert(tid);
        if last_ids.len() < 200 {
            last_ids.push((message_id, subject));
        } else {
            let k = rng.below(200);
            last_ids[k] = (message_id, subject);
        }
    }
    ingest::refresh_threads(&pool, &touched).await?;
    for f in &folders {
        ingest::refresh_folder_unread(&pool, *f).await?;
    }
    Ok(())
}

/// Dépose dans la boîte de réception d'un compte fictif `photos@example.com` (hôtes invalides : aucune synchro) une conversation avec des images jointes (paysage, portrait,
/// carré, une image trop grosse pour l'aperçu) et un PDF, à partir des fichiers de `images_dir` : l'affichage des
/// pièces jointes dans le fil se juge sur de vrais messages. Passe par `ingest`, comme un message reçu.
pub async fn seed_photo_thread(data_dir: &Path, images_dir: &Path) -> Result<usize> {
    use crate::mail::ingest;
    let read = |name: &str| {
        std::fs::read(images_dir.join(name)).map_err(|e| invalid(format!("{name} : {e}")))
    };
    let kind = |name: &str| -> Result<ContentType> {
        let mime = match name.rsplit('.').next() {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("png") => "image/png",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("pdf") => "application/pdf",
            _ => "application/octet-stream",
        };
        ContentType::parse(mime).map_err(invalid)
    };
    struct Msg {
        id: &'static str,
        from_me: bool,
        text: &'static str,
        hours_ago: u64,
        files: &'static [&'static str],
        reply_to: Option<&'static str>,
    }
    let thread = [
        Msg { id: "photos-1", from_me: false, text: "Salut ! Voilà les photos du chantier de ce matin : la vue d'ensemble, l'escalier, et un détail de la rambarde.\n\nDis-moi ce que tu en penses.\n\nValentino", hours_ago: 30, files: &["chantier-vue.jpg", "chantier-portrait.png", "chantier-carre.jpg"], reply_to: None },
        Msg { id: "photos-2", from_me: true, text: "Merci Valentino, elles sont très nettes. Peux-tu m'envoyer la façade et le plan à jour ?\n\nAlex", hours_ago: 26, files: &[], reply_to: Some("photos-1") },
        Msg { id: "photos-3", from_me: false, text: "Voici la façade, le plan en PDF, et la photo en pleine résolution (elle est lourde).\n\nValentino", hours_ago: 20, files: &["chantier-vue.jpg", "chantier-portrait.png", "chantier-carre.jpg", "facade.jpg", "plan.pdf", "gros-fichier.png"], reply_to: Some("photos-2") },
    ];
    let pool = crate::db::init(&crate::db::db_path(data_dir)).await?;
    let dir = data_dir.join("attachments");
    let raw_dir = data_dir.join("raw");
    sqlx::query("INSERT OR IGNORE INTO accounts (id, email, username, imap_host, smtp_host) VALUES ('photos','alex.morel@example.com','alex','invalid.invalid','invalid.invalid')")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO folders (account_id, path, name, role) VALUES ('photos', 'INBOX', 'INBOX', 'inbox')")
        .execute(&pool)
        .await?;
    let folder: i64 =
        sqlx::query_scalar("SELECT id FROM folders WHERE account_id = 'photos' AND path = 'INBOX'")
            .fetch_one(&pool)
            .await?;
    let valentino: Mailbox = "Valentino Rossi <valentino.rossi@example.com>"
        .parse()
        .map_err(invalid)?;
    let me: Mailbox = "Alex Morel <alex.morel@example.com>"
        .parse()
        .map_err(invalid)?;
    let now = SystemTime::now();
    let mut touched = std::collections::HashSet::new();
    for Msg {
        id,
        from_me,
        text,
        hours_ago,
        files,
        reply_to,
    } in &thread
    {
        let (from, to) = if *from_me {
            (&me, &valentino)
        } else {
            (&valentino, &me)
        };
        let at = now - Duration::from_secs(hours_ago * 3600);
        let mut b = Message::builder()
            .from(from.clone())
            .to(to.clone())
            .subject(if reply_to.is_some() {
                "Re: Photos du chantier"
            } else {
                "Photos du chantier"
            })
            .date(at)
            .message_id(Some(format!("<{id}@example.com>")));
        if let Some(parent) = reply_to {
            let parent = format!("<{parent}@example.com>");
            b = b.in_reply_to(parent.clone()).references(parent);
        }
        let mut parts = MultiPart::mixed().singlepart(SinglePart::plain((*text).to_string()));
        for f in *files {
            parts = parts.singlepart(Attachment::new((*f).to_string()).body(read(f)?, kind(f)?));
        }
        let raw = b.multipart(parts).map_err(invalid)?.formatted();
        let parsed = ingest::parse(&raw, None).ok_or_else(|| invalid("message illisible"))?;
        // Comme un message reçu : le brut est rangé dans le magasin, les pièces jointes sont extraites à la demande.
        let hash = crate::mail::rawstore::put(&raw_dir, &raw).await?;
        let tid = ingest::store_with(
            &pool,
            &dir,
            ingest::MessageLoc {
                account_id: "photos",
                folder_id: folder,
                uid: ingest::next_uid(&pool, folder).await?,
                size: Some(raw.len() as i64),
            },
            ingest::Flags {
                seen: true, // lus : aucune action IMAP à l'ouverture
                flagged: false,
            },
            parsed,
            ingest::StoreOpts {
                raw_hash: Some(&hash),
                lazy_attachments: true,
            },
        )
        .await?;
        touched.insert(tid);
    }
    ingest::refresh_threads(&pool, &touched).await?;
    ingest::refresh_folder_unread(&pool, folder).await?;
    Ok(thread.len())
}

/// Archive en local la première conversation de la boîte de réception dont l'objet contient
/// `subject` (essai de bout en bout : copie vérifiée, puis suppression sur le serveur).
pub async fn archive_local_by_subject(data_dir: &Path, subject: &str) -> Result<String> {
    use crate::mail::{account, local};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pool = crate::db::init(&crate::db::db_path(data_dir)).await?;
    let (thread_id, folder_id, title): (i64, i64, String) = sqlx::query_as(
        "SELECT m.thread_id, m.folder_id, m.subject FROM messages m JOIN folders f ON f.id = m.folder_id \
         WHERE f.role = ?2 AND m.thread_id IS NOT NULL AND m.subject LIKE ?1 ESCAPE '\\' ORDER BY m.date DESC LIMIT 1",
    )
    .bind(envelope_core::search::like_contains(subject))
    .bind(FolderRole::Inbox)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| AppError::NotFound((format!("aucun message « {subject} » dans la boîte de réception")).into()))?;
    let target: i64 =
        sqlx::query_scalar("SELECT id FROM folders WHERE account_id = ? AND name = 'Archives'")
            .bind(account::LOCAL_ID)
            .fetch_one(&pool)
            .await?;
    local::archive(
        &pool,
        &data_dir.join("attachments"),
        &data_dir.join("raw"),
        thread_id,
        folder_id,
        target,
    )
    .await?;
    Ok(title)
}

/// Nombre de messages de la boîte de réception **du serveur** dont l'objet contient `subject`.
pub async fn server_inbox_count(data_dir: &Path, subject: &str) -> Result<usize> {
    use crate::mail::{account, imap};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pool = crate::db::init(&crate::db::db_path(data_dir)).await?;
    let acct = account::list(&pool)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("compte".into()))?;
    let cred = account::resolve_credential(&acct).await?;
    let mut session = imap::connect(&acct, &cred).await?;
    session.select("INBOX").await?;
    let found = session
        .uid_search(format!("SUBJECT \"{}\"", subject.replace('"', "")))
        .await?;
    let _ = session.logout().await;
    Ok(found.len())
}

/// Importe la source dans une **base temporaire** (jamais la vraie) et décrit le résultat :
/// contrôle d'un profil Thunderbird avant de l'essayer dans l'application.
pub async fn import_check(root: &Path) -> Result<String> {
    use crate::mail::mbox;
    let base = std::env::temp_dir().join(format!("envelope-import-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let pool = crate::db::init(&base.join("check.db")).await?;
    let found = mbox::discover(root).await?;
    let mut out = String::from("Dossiers trouvés :\n");
    for f in &found {
        out += &format!(
            "  {} {} ({:?}, {})\n",
            if f.selected { "[x]" } else { "[ ]" },
            f.name,
            f.kind,
            f.size
        );
    }
    let keys = found
        .iter()
        .filter(|f| f.selected)
        .map(|f| f.key.clone())
        .collect();
    let report = mbox::import(
        &pool,
        &base.join("att"),
        &base.join("raw"),
        root,
        &keys,
        &|_| {},
    )
    .await?;
    out += &format!("Rapport : {report:?}\n");
    let rows: Vec<(String, String, bool, bool, i64)> = sqlx::query_as(
        "SELECT f.name, m.subject, m.is_read, m.is_starred, (SELECT COUNT(*) FROM attachments a WHERE a.message_id = m.id) \
         FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.account_id = 'local' ORDER BY f.name, m.date",
    )
    .fetch_all(&pool)
    .await?;
    for (folder, subject, read, star, atts) in rows {
        out += &format!("  [{folder}] {subject}  lu={read} suivi={star} pj={atts}\n");
    }
    // Aller-retour : export mbox de chaque dossier puis relecture ; les messages doivent être identiques
    // (seules les lignes « From » du corps sont échappées par le format).
    let folders: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM folders WHERE account_id = 'local' AND id IN (SELECT folder_id FROM messages)")
        .fetch_all(&pool)
        .await?;
    for (id, name) in folders {
        let dest = base.join(format!("{id}.mbox"));
        let r =
            crate::mail::export::export_mbox(&pool, &base.join("raw"), id, &dest, &|_| {}).await?;
        let mut reader = crate::mail::mbox::MboxReader::new(tokio::fs::File::open(&dest).await?);
        let (mut back, mut same) = (0usize, 0usize);
        while let Some(item) = reader.next().await? {
            back += 1;
            let hash = crate::mail::rawstore::put(&base.join("raw2"), &item.raw).await?;
            if crate::mail::rawstore::get(&base.join("raw"), &hash)
                .await
                .is_ok()
            {
                same += 1;
            }
        }
        out += &format!("Aller-retour [{name}] : {} exportés, {back} relus, {same} identiques à l'original ({} avec échappement « From »)\n", r.exported, back - same);
    }
    let _ = std::fs::remove_dir_all(&base);
    Ok(out)
}

/// Test de connexion et d'authentification SMTP du premier compte (aucun message n'est envoyé).
pub async fn smtp_check(data_dir: &Path) -> Result<()> {
    use crate::mail::{account, smtp};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pool = crate::db::init(&crate::db::db_path(data_dir)).await?;
    let acct = account::list(&pool)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("compte".into()))?;
    let cred = account::resolve_credential(&acct).await?;
    smtp::test(&acct, &cred).await
}
