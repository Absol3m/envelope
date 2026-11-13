//! Envoi SMTP via lettre (TLS implicite, STARTTLS ou clair ; mot de passe ou XOAUTH2).

use base64::{engine::general_purpose::STANDARD, Engine};
use lettre::message::{header, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, Credential, Security};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMessage {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    /// Message-ID du message auquel on répond (pour le threading chez les destinataires).
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<OutgoingAttachment>,
    /// Brouillon dont ce message est la version finale : il est supprimé du serveur une fois l'envoi réussi.
    #[serde(default)]
    pub draft_id: Option<String>,
    /// Adresse d'expéditeur : celle du compte (défaut) ou l'un de ses alias.
    #[serde(default)]
    pub from_email: Option<String>,
}

/// Pièce jointe fournie par l'interface (contenu en base64).
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingAttachment {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

/// Taille cumulée maximale des pièces jointes (limite courante des serveurs : 25 Mo).
/// À garder alignée avec `MAX_ATTACHMENTS_BYTES` côté interface (src/lib/compose.ts).
const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;
/// Idem : `MAX_ATTACHMENTS` côté interface.
const MAX_ATTACHMENTS: usize = 20;

/// Nom de fichier sans chemin ni caractères de contrôle (jamais d'injection d'en-tête).
fn header_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != '"')
        .collect();
    match clean.trim() {
        "" => "pièce-jointe".into(),
        trimmed => trimmed.to_string(),
    }
}

fn attachment_parts(list: &[OutgoingAttachment]) -> Result<Vec<SinglePart>> {
    if list.len() > MAX_ATTACHMENTS {
        return Err(AppError::Invalid(msg!(
            "smtp.too_many_attachments",
            "too many attachments (maximum {max})",
            max = MAX_ATTACHMENTS
        )));
    }
    let mut total = 0usize;
    let mut parts = Vec::with_capacity(list.len());
    for a in list {
        let data = STANDARD.decode(a.data_base64.trim()).map_err(|e| {
            AppError::Invalid(msg!(
                "smtp.attachment_unreadable",
                "attachment “{name}” unreadable: {detail}",
                name = a.filename,
                detail = e
            ))
        })?;
        total += data.len();
        if total > MAX_ATTACHMENTS_BYTES {
            return Err(AppError::Invalid(msg!(
                "smtp.attachments_too_big",
                "attachments too large (maximum {max} MB in total)",
                max = MAX_ATTACHMENTS_BYTES / 1024 / 1024
            )));
        }
        let ct = header::ContentType::parse(&a.mime_type)
            .or_else(|_| header::ContentType::parse("application/octet-stream"))
            .map_err(smtp_err)?;
        parts.push(Attachment::new(header_filename(&a.filename)).body(data, ct));
    }
    Ok(parts)
}

fn smtp_err(e: impl std::fmt::Display) -> AppError {
    AppError::Smtp((e.to_string()).into())
}

/// Échec d'un envoi : « injoignable » quand le serveur n'a rien répondu (connexion, réseau, délai) — le message n'est
/// pas parti et peut être renvoyé tel quel ; toute réponse du serveur (4xx, 5xx, authentification, TLS refusé) est
/// une erreur à montrer, pas à retenter en silence.
fn send_err(e: lettre::transport::smtp::Error) -> AppError {
    let answered =
        e.is_response() || e.is_client() || e.is_transient() || e.is_permanent() || e.is_tls();
    if answered {
        return smtp_err(e);
    }
    AppError::Unreachable(msg!(
        "smtp.unreachable",
        "the SMTP server can't be reached: {detail}",
        detail = e
    ))
}

fn mailbox(s: &str) -> Result<Mailbox> {
    s.parse().map_err(|e| {
        AppError::Invalid(msg!(
            "address.invalid_named",
            "invalid address “{address}”: {detail}",
            address = s,
            detail = e
        ))
    })
}

fn bracket(id: &str) -> String {
    format!("<{}>", id.trim_matches(|c| c == '<' || c == '>'))
}

/// En-tête qui identifie un brouillon d'Envelope sur le serveur (voir `mail::drafts`).
pub const DRAFT_ID_HEADER: &str = "X-Envelope-Draft-Id";
/// Longueur maximale d'un identifiant de brouillon (un UUID en fait 36) ; au-delà, l'ingestion l'ignorerait.
pub const MAX_DRAFT_ID: usize = 80;
#[derive(Clone, Debug, PartialEq)]
struct DraftId(String);

impl header::Header for DraftId {
    fn name() -> header::HeaderName {
        header::HeaderName::new_from_ascii_str(DRAFT_ID_HEADER)
    }
    fn parse(s: &str) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self(s.trim().to_string()))
    }
    fn display(&self) -> header::HeaderValue {
        header::HeaderValue::new(Self::name(), self.0.clone())
    }
}

pub fn build_message(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    build(account, out, false)
}

/// Message de brouillon : comme un message à envoyer, mais les destinataires sont facultatifs, Bcc est
/// conservé et l'identifiant du brouillon est inscrit dans un en-tête.
pub fn build_draft(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    build(account, out, true)
}

fn build(account: &Account, out: &OutgoingMessage, draft: bool) -> Result<Message> {
    let from = account::sender(account, out.from_email.as_deref())?;
    let me = from.email.clone();
    let mut b = Message::builder().from(from).subject(out.subject.clone());
    if draft {
        let id = out
            .draft_id
            .as_deref()
            .map(str::trim)
            .filter(|i| !i.is_empty() && i.len() <= MAX_DRAFT_ID)
            .ok_or_else(|| {
                AppError::Invalid(msg!(
                    "draft.id_invalid",
                    "draft identifier missing or too long"
                ))
            })?;
        b = b.keep_bcc().header(DraftId(id.to_string()));
        if out.to.is_empty() && out.cc.is_empty() && out.bcc.is_empty() {
            // Jamais envoyé : l'enveloppe SMTP exige pourtant un destinataire, on met l'expéditeur.
            b = b.envelope(
                lettre::address::Envelope::new(Some(me.clone()), vec![me]).map_err(smtp_err)?,
            );
        }
    } else if out.to.is_empty() {
        return Err(AppError::Invalid(msg!(
            "smtp.no_recipient",
            "at least one recipient is required"
        )));
    }
    for a in &out.to {
        b = b.to(mailbox(a)?);
    }
    for a in &out.cc {
        b = b.cc(mailbox(a)?);
    }
    for a in &out.bcc {
        b = b.bcc(mailbox(a)?);
    }
    if let Some(id) = &out.in_reply_to {
        b = b.in_reply_to(bracket(id));
    }
    if !out.references.is_empty() {
        b = b.references(
            out.references
                .iter()
                .map(|r| bracket(r))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    let attachments = attachment_parts(&out.attachments)?;
    let text = SinglePart::builder()
        .header(header::ContentType::TEXT_PLAIN)
        .body(out.body_text.clone());
    let alternative =
        |html: &str| MultiPart::alternative_plain_html(out.body_text.clone(), html.to_owned());
    let msg = if attachments.is_empty() {
        match &out.body_html {
            Some(html) => b.multipart(alternative(html)),
            None => b.singlepart(text),
        }
    } else {
        let body = match &out.body_html {
            Some(html) => MultiPart::mixed().multipart(alternative(html)),
            None => MultiPart::mixed().singlepart(text),
        };
        b.multipart(attachments.into_iter().fold(body, MultiPart::singlepart))
    };
    msg.map_err(smtp_err)
}

fn transport(account: &Account, cred: &Credential) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = account.smtp_host.as_str();
    let builder = match Security::parse(&account.smtp_security) {
        Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(host).map_err(smtp_err)?,
        Security::Starttls => {
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host).map_err(smtp_err)?
        }
        // Le mot de passe partirait en clair : refusé (comme pour IMAP).
        Security::None => {
            return Err(AppError::Invalid(msg!(
                "smtp.unencrypted",
                "SMTP: unencrypted connection refused (choose TLS or STARTTLS)"
            )))
        }
    }
    .port(account.smtp_port as u16);

    let builder = match cred {
        Credential::Password { user, password } => {
            builder.credentials(Credentials::new(user.clone(), password.clone()))
        }
        Credential::OAuth { user, access_token } => builder
            .credentials(Credentials::new(user.clone(), access_token.clone()))
            .authentication(vec![Mechanism::Xoauth2]),
    };
    Ok(builder.build())
}

/// Envoie le message et renvoie sa forme brute (pour en garder une copie dans « Envoyés »).
pub async fn send(account: &Account, cred: &Credential, out: &OutgoingMessage) -> Result<Vec<u8>> {
    let msg = build_message(account, out)?;
    let raw = msg.formatted();
    transport(account, cred)?
        .send(msg)
        .await
        .map_err(send_err)?;
    Ok(raw)
}

/// Vérifie la connexion + l'authentification SMTP sans envoyer de message.
pub async fn test(account: &Account, cred: &Credential) -> Result<()> {
    let ok = transport(account, cred)?
        .test_connection()
        .await
        .map_err(smtp_err)?;
    if ok {
        Ok(())
    } else {
        Err(AppError::Smtp(msg!(
            "smtp.not_accepted",
            "the SMTP server did not reply positively"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> Account {
        Account {
            id: "a".into(),
            email: "me@x.fr".into(),
            display_name: Some("Moi".into()),
            provider: "imap".into(),
            auth_kind: "password".into(),
            username: "me@x.fr".into(),
            imap_host: "i".into(),
            imap_port: 993,
            imap_security: "tls".into(),
            smtp_host: "s".into(),
            smtp_port: 465,
            smtp_security: "tls".into(),
            last_sync_at: None,
            sync_status: "idle".into(),
            sync_error: None,
            signature: String::new(),
            copy_sent: true,
            pending_copies: 0,
            pending_copy_error: None,
            identities: vec![],
        }
    }

    fn outgoing(attachments: Vec<OutgoingAttachment>, html: Option<&str>) -> OutgoingMessage {
        OutgoingMessage {
            to: vec!["bob@x.fr".into()],
            cc: vec![],
            bcc: vec![],
            subject: "Sujet".into(),
            body_text: "Bonjour".into(),
            body_html: html.map(Into::into),
            in_reply_to: None,
            references: vec![],
            attachments,
            draft_id: None,
            from_email: None,
        }
    }

    /// Un port fermé n'est pas un refus du serveur : le message pourra repartir tel quel (boîte d'envoi).
    #[tokio::test]
    async fn serveur_injoignable_distingue_d_un_refus() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener); // plus personne n'écoute
        let acct = Account {
            smtp_host: "127.0.0.1".into(),
            smtp_port: i64::from(port),
            smtp_security: "starttls".into(),
            ..account()
        };
        let cred = Credential::Password {
            user: "me@x.fr".into(),
            password: "x".into(),
        };
        let err = send(&acct, &cred, &outgoing(vec![], None))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unreachable(_)), "{err:?}");
        // Une erreur de construction (destinataire invalide) n'est jamais « injoignable ».
        let mut bad = outgoing(vec![], None);
        bad.to = vec!["pas une adresse".into()];
        let err = send(&acct, &cred, &bad).await.unwrap_err();
        assert!(!matches!(err, AppError::Unreachable(_)));
    }

    fn att(name: &str, bytes: &[u8]) -> OutgoingAttachment {
        OutgoingAttachment {
            filename: name.into(),
            mime_type: "application/pdf".into(),
            data_base64: STANDARD.encode(bytes),
        }
    }

    fn raw(out: &OutgoingMessage) -> String {
        String::from_utf8_lossy(&build_message(&account(), out).unwrap().formatted()).into_owned()
    }

    #[test]
    fn message_with_attachment_is_multipart_mixed() {
        let r = raw(&outgoing(
            vec![att("devis.pdf", &[0u8, 255, 1, 2, 3, 200])],
            Some("<p>Bonjour</p>"),
        ));
        assert!(r.contains("multipart/mixed"));
        assert!(r.contains("multipart/alternative"));
        assert!(r.contains("filename=\"devis.pdf\""));
        assert!(r.contains(&STANDARD.encode([0u8, 255, 1, 2, 3, 200])));
    }

    #[test]
    fn plain_message_stays_simple() {
        let r = raw(&outgoing(vec![], None));
        assert!(!r.contains("multipart"));
    }

    #[test]
    fn filename_is_sanitized() {
        assert_eq!(header_filename("../../etc/passwd"), "passwd");
        assert_eq!(header_filename("a\"b\r\n.pdf"), "ab.pdf");
        assert_eq!(header_filename("  "), "pièce-jointe");
    }

    #[test]
    fn expediteur_alias_dans_l_en_tete_et_refus_d_une_adresse_libre() {
        let mut a = account();
        a.identities.push(account::Identity {
            id: 1,
            name: "Pro".into(),
            email: "contact@x.fr".into(),
        });
        let mut out = outgoing(vec![], None);
        out.from_email = Some("contact@x.fr".into());
        let text =
            String::from_utf8_lossy(&build_message(&a, &out).unwrap().formatted()).to_string();
        assert!(text.contains("From: Pro <contact@x.fr>"), "{text}");
        out.from_email = Some("usurpe@autre.fr".into());
        assert!(build_message(&a, &out).is_err());
    }

    #[test]
    fn rejects_oversize_and_garbage() {
        let big = vec![0u8; MAX_ATTACHMENTS_BYTES + 1];
        assert!(build_message(&account(), &outgoing(vec![att("big.bin", &big)], None)).is_err());
        let bad = OutgoingAttachment {
            filename: "x".into(),
            mime_type: "text/plain".into(),
            data_base64: "!!!".into(),
        };
        assert!(build_message(&account(), &outgoing(vec![bad], None)).is_err());
    }
}
