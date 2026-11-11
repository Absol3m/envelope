//! Envoi SMTP via lettre (TLS implicite, STARTTLS ou clair ; mot de passe ou XOAUTH2).

use base64::{engine::general_purpose::STANDARD, Engine};
use lettre::message::{header, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;

use crate::error::{AppError, Result};
use crate::mail::account::{Account, Credential, Security};

#[derive(Debug, Deserialize)]
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
}

/// Pièce jointe fournie par l'interface (contenu en base64).
#[derive(Debug, Deserialize)]
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
        return Err(AppError::Invalid(format!(
            "trop de pièces jointes (maximum {MAX_ATTACHMENTS})"
        )));
    }
    let mut total = 0usize;
    let mut parts = Vec::with_capacity(list.len());
    for a in list {
        let data = STANDARD.decode(a.data_base64.trim()).map_err(|e| {
            AppError::Invalid(format!("pièce jointe « {} » illisible : {e}", a.filename))
        })?;
        total += data.len();
        if total > MAX_ATTACHMENTS_BYTES {
            return Err(AppError::Invalid(format!(
                "pièces jointes trop volumineuses (maximum {} Mo au total)",
                MAX_ATTACHMENTS_BYTES / 1024 / 1024
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
    AppError::Smtp(e.to_string())
}

fn mailbox(s: &str) -> Result<Mailbox> {
    s.parse()
        .map_err(|e| AppError::Invalid(format!("adresse invalide « {s} » : {e}")))
}

fn bracket(id: &str) -> String {
    format!("<{}>", id.trim_matches(|c| c == '<' || c == '>'))
}

pub fn build_message(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    let from = match &account.display_name {
        Some(n) if !n.is_empty() => format!("{n} <{}>", account.email),
        _ => account.email.clone(),
    };
    let mut b = Message::builder()
        .from(mailbox(&from)?)
        .subject(out.subject.clone());
    if out.to.is_empty() {
        return Err(AppError::Invalid(
            "au moins un destinataire est requis".into(),
        ));
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
        Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
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
        .map_err(smtp_err)?;
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
        Err(AppError::Smtp(
            "le serveur SMTP n'a pas répondu positivement".into(),
        ))
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
        }
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
