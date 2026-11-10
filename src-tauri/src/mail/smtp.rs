//! Envoi SMTP via lettre (TLS implicite, STARTTLS ou clair ; mot de passe ou XOAUTH2).

use lettre::message::{header, Mailbox, MultiPart, SinglePart};
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
    let msg = match &out.body_html {
        Some(html) => b.multipart(MultiPart::alternative_plain_html(
            out.body_text.clone(),
            html.clone(),
        )),
        None => b.singlepart(
            SinglePart::builder()
                .header(header::ContentType::TEXT_PLAIN)
                .body(out.body_text.clone()),
        ),
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
