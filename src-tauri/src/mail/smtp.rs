//! Envoi SMTP via lettre (TLS implicite, STARTTLS ou clair ; mot de passe ou XOAUTH2).

use base64::{engine::general_purpose::STANDARD, Engine};
use lettre::message::{header, Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Account, Credential, Security};

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, optional_fields)]
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
    /// Signer (OpenPGP) avec la clé associée à l'adresse d'expéditeur.
    #[serde(default)]
    pub pgp_sign: bool,
    /// Chiffrer (OpenPGP) pour tous les destinataires et pour l'expéditeur.
    #[serde(default)]
    pub pgp_encrypt: bool,
}

impl OutgoingMessage {
    /// Le message doit-il être signé ou chiffré ?
    pub fn wants_pgp(&self) -> bool {
        self.pgp_sign || self.pgp_encrypt
    }
}

/// Pièce jointe fournie par l'interface (contenu en base64).
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export)]
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
        "" => "attachment".into(),
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

/// « Injoignable » : le serveur n'a rien répondu (connexion, réseau, délai), le message peut être renvoyé tel quel ;
/// toute réponse (4xx, 5xx, authentification, TLS) est une erreur à montrer. lettre n'expose pas ces cas : on écarte ce
/// qui est une réponse, le reste est « sans réponse » (figé par les essais du port fermé et de l'adresse invalide).
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

/// Contrôles d'un envoi à venir (envoi programmé) : ceux de `build` (expéditeur autorisé, destinataire, adresses) sans
/// construire le corps ni les pièces jointes.
pub fn check_sendable(account: &Account, out: &OutgoingMessage) -> Result<()> {
    headers(account, out, false).map(drop)
}

pub fn build_message(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    build(account, out, false)
}

/// Message de brouillon : comme un message à envoyer, mais les destinataires sont facultatifs, Bcc est
/// conservé et l'identifiant du brouillon est inscrit dans un en-tête.
pub fn build_draft(account: &Account, out: &OutgoingMessage) -> Result<Message> {
    build(account, out, true)
}

/// En-têtes du message et contrôles de l'envoi ; le corps et les pièces jointes s'ajoutent dans `build`.
fn headers(account: &Account, out: &OutgoingMessage, draft: bool) -> Result<MessageBuilder> {
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
    Ok(b)
}

/// Corps du message (texte, HTML, pièces jointes) : une partie simple ou un multipart.
enum Body {
    Single(SinglePart),
    Multi(MultiPart),
}

impl Body {
    fn formatted(&self) -> Vec<u8> {
        match self {
            Body::Single(p) => p.formatted(),
            Body::Multi(p) => p.formatted(),
        }
    }
    fn into_message(self, b: MessageBuilder) -> Result<Message> {
        match self {
            Body::Single(p) => b.singlepart(p),
            Body::Multi(p) => b.multipart(p),
        }
        .map_err(smtp_err)
    }
    fn add_to(self, b: lettre::message::MultiPartBuilder) -> lettre::message::MultiPart {
        match self {
            Body::Single(p) => b.singlepart(p),
            Body::Multi(p) => b.multipart(p),
        }
    }
}

fn body(out: &OutgoingMessage) -> Result<Body> {
    let attachments = attachment_parts(&out.attachments)?;
    let text = SinglePart::builder()
        .header(header::ContentType::TEXT_PLAIN)
        .body(out.body_text.clone());
    let alternative =
        |html: &str| MultiPart::alternative_plain_html(out.body_text.clone(), html.to_owned());
    Ok(if attachments.is_empty() {
        match &out.body_html {
            Some(html) => Body::Multi(alternative(html)),
            None => Body::Single(text),
        }
    } else {
        let body = match &out.body_html {
            Some(html) => MultiPart::mixed().multipart(alternative(html)),
            None => MultiPart::mixed().singlepart(text),
        };
        Body::Multi(attachments.into_iter().fold(body, MultiPart::singlepart))
    })
}

fn build(account: &Account, out: &OutgoingMessage, draft: bool) -> Result<Message> {
    headers(account, out, draft).and_then(|b| body(out)?.into_message(b))
}

/// Ce qu'il faut pour signer et chiffrer un message (voir `pgp_send`).
pub struct Seal {
    /// Clé de signature et sa phrase secrète, si on signe.
    pub signer: Option<(pgp::composed::SignedSecretKey, Option<String>)>,
    /// Destinataires du chiffrement (vide : on ne chiffre pas).
    pub recipients: Vec<crate::openpgp::PublicKey>,
}

fn seal_err(e: crate::openpgp::SealError) -> AppError {
    use crate::openpgp::SealError;
    match e {
        SealError::Passphrase => AppError::Invalid(msg!(
            "pgp.passphrase_required",
            "the signing key is locked: its passphrase is missing or wrong"
        )),
        SealError::NoEncryptionKey => AppError::Invalid(msg!(
            "pgp.no_encryption_subkey",
            "a recipient's key has no encryption subkey"
        )),
        SealError::Failed => AppError::Invalid(msg!(
            "pgp.seal_failed",
            "could not sign or encrypt the message"
        )),
    }
}

fn pgp_part(content_type: &str, body: String) -> Result<SinglePart> {
    let ct = header::ContentType::parse(content_type).map_err(smtp_err)?;
    Ok(SinglePart::builder().header(ct).body(body))
}

/// Message PGP/MIME (RFC 3156) : signé (`multipart/signed`), puis chiffré (`multipart/encrypted`) si demandé. La
/// signature porte sur le corps tel qu'il sera écrit, sans le CRLF qui précède la frontière (il lui appartient).
pub fn build_sealed(account: &Account, out: &OutgoingMessage, seal: &Seal) -> Result<Message> {
    let b = headers(account, out, false)?;
    let mut inner = body(out)?;
    let encrypt = !seal.recipients.is_empty();
    if let (Some((key, passphrase)), false) = (&seal.signer, encrypt) {
        // Signé seulement : `multipart/signed`, signature détachée.
        let formatted = inner.formatted();
        let content = formatted.strip_suffix(b"\r\n").unwrap_or(&formatted);
        let signature =
            crate::openpgp::sign_detached(content, key, passphrase.as_deref()).map_err(seal_err)?;
        let signed = inner.add_to(MultiPart::signed(
            "application/pgp-signature".into(),
            "pgp-sha256".into(),
        ));
        inner = Body::Multi(signed.singlepart(pgp_part("application/pgp-signature", signature)?));
    }
    if encrypt {
        // Chiffré (et signé) : la signature voyage dans le message chiffré, une seule enveloppe OpenPGP.
        let recipients: Vec<_> = seal.recipients.iter().collect();
        let signer = seal.signer.as_ref().map(|(k, p)| (k, p.as_deref()));
        let armored = crate::openpgp::encrypt_to(&inner.formatted(), &recipients, signer)
            .map_err(seal_err)?;
        let encrypted = MultiPart::encrypted("application/pgp-encrypted".into())
            .singlepart(pgp_part("application/pgp-encrypted", "Version: 1".into())?)
            .singlepart(pgp_part("application/octet-stream", armored)?);
        inner = Body::Multi(encrypted);
    }
    inner.into_message(b)
}

fn transport(account: &Account, cred: &Credential) -> Result<AsyncSmtpTransport<Tokio1Executor>> {
    let host = account.smtp_host.as_str();
    let builder = match account.smtp_security {
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
    send_message(account, cred, build_message(account, out)?).await
}

/// Envoie un message construit par `build_message`.
async fn send_message(account: &Account, cred: &Credential, msg: Message) -> Result<Vec<u8>> {
    let raw = msg.formatted();
    transport(account, cred)?
        .send(msg)
        .await
        .map_err(send_err)?;
    Ok(raw)
}

/// Message déjà construit (signé, chiffré) prêt à partir : MIME brut et enveloppe SMTP (qui garde le Bcc, absent du MIME).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub raw: Vec<u8>,
    pub from: Option<String>,
    pub to: Vec<String>,
}

impl Sealed {
    pub fn of(message: &Message) -> Self {
        let envelope = message.envelope();
        Self {
            raw: message.formatted(),
            from: envelope.from().map(ToString::to_string),
            to: envelope.to().iter().map(ToString::to_string).collect(),
        }
    }

    fn envelope(&self) -> Result<lettre::address::Envelope> {
        let from = self
            .from
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(smtp_err)?;
        let to = self
            .to
            .iter()
            .map(|a| a.parse())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(smtp_err)?;
        lettre::address::Envelope::new(from, to).map_err(smtp_err)
    }
}

/// Envoie un message déjà construit, tel quel ; rend sa forme brute (copie dans « Envoyés »).
pub async fn send_sealed(account: &Account, cred: &Credential, sealed: &Sealed) -> Result<Vec<u8>> {
    transport(account, cred)?
        .send_raw(&sealed.envelope()?, &sealed.raw)
        .await
        .map_err(send_err)?;
    Ok(sealed.raw.clone())
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
            username: "me@x.fr".into(),
            ..Account::fixture()
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
            pgp_sign: false,
            pgp_encrypt: false,
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
            smtp_security: Security::Starttls,
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
        assert_eq!(header_filename("  "), "attachment");
    }

    #[test]
    fn controle_d_un_envoi_a_venir_sans_construire_le_message() {
        let a = account();
        assert!(check_sendable(&a, &outgoing(vec![], None)).is_ok());
        let mut sans_destinataire = outgoing(vec![], None);
        sans_destinataire.to.clear();
        assert!(check_sendable(&a, &sans_destinataire).is_err());
        let mut adresse = outgoing(vec![], None);
        adresse.cc.push("pas une adresse".into());
        assert!(check_sendable(&a, &adresse).is_err());
        let mut libre = outgoing(vec![], None);
        libre.from_email = Some("inconnu@ailleurs.fr".into());
        assert!(check_sendable(&a, &libre).is_err());
    }

    #[test]
    fn expediteur_alias_dans_l_en_tete_et_refus_d_une_adresse_libre() {
        let mut a = account();
        a.identities.push(account::Identity {
            id: 1,
            name: "Pro".into(),
            email: "contact@x.fr".into(),
            signature: String::new(),
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

    /// Relit ce que `build_sealed` écrit avec le code de lecture (`openpgp`) : signature valide, déchiffrement, texte intact.
    mod sealed {
        use super::*;
        use crate::openpgp::tests::keygen;
        use crate::openpgp::{
            decrypt, public_of, shape, verify_detached, SecretKey, Shape, SigState,
        };

        fn attachment() -> OutgoingAttachment {
            OutgoingAttachment {
                filename: "a.txt".into(),
                mime_type: "text/plain".into(),
                data_base64: STANDARD.encode("pièce jointe\nsur deux lignes"),
            }
        }

        fn signed_valid(raw: &[u8], key: &pgp::composed::SignedSecretKey) -> bool {
            let Some(Shape::Signed { content, signature }) = shape(raw) else {
                return false;
            };
            verify_detached(&signature, &content, &[public_of(key)])
                .is_some_and(|v| v.state == SigState::Valid)
        }

        #[test]
        fn signe_un_texte_simple_un_html_et_des_pieces_jointes() {
            let key = keygen("Moi <me@x.fr>", None);
            let seal = Seal {
                signer: Some((key.clone(), None)),
                recipients: vec![],
            };
            for (html, attachments) in [
                (None, vec![]),
                (Some("<p>Bonjour <b>Bob</b></p>"), vec![]),
                (None, vec![attachment()]),
                (Some("<p>Salut é</p>"), vec![attachment()]),
            ] {
                let out = outgoing(attachments, html);
                let raw = build_sealed(&account(), &out, &seal).unwrap().formatted();
                assert!(signed_valid(&raw, &key), "html={html:?}");
            }
        }

        #[test]
        fn chiffre_pour_tous_et_pour_soi_puis_se_relit() {
            let me = keygen("Moi <me@x.fr>", None);
            let bob = keygen("Bob <bob@x.fr>", Some("sésame"));
            let eve = keygen("Eve <eve@x.fr>", None);
            let seal = Seal {
                signer: Some((me.clone(), None)),
                recipients: vec![public_of(&bob), public_of(&me)],
            };
            let mut out = outgoing(vec![attachment()], Some("<p>Secret</p>"));
            out.bcc = vec!["caché@x.fr".into()];
            let raw = build_sealed(&account(), &out, &seal).unwrap().formatted();
            let text = String::from_utf8_lossy(&raw);
            // Rien de lisible dans le message, et le Bcc n'apparaît pas dans les en-têtes.
            assert!(!text.contains("Bonjour") && !text.contains("Secret"));
            assert!(!text.contains("caché@x.fr"));
            assert!(text.contains("multipart/encrypted"));

            let Some(Shape::Encrypted { armored, mime }) = shape(&raw) else {
                panic!("forme chiffrée attendue")
            };
            assert!(mime);
            let read = |k: &pgp::composed::SignedSecretKey, pw: Option<&str>| {
                decrypt(
                    &armored,
                    &[SecretKey {
                        key: k.clone(),
                        fingerprint: String::new(),
                        passphrase: pw.map(str::to_string),
                    }],
                    &[public_of(&me)],
                )
            };
            // Bob (clé verrouillée) et nous-mêmes relisons ; Eve non.
            let for_bob = read(&bob, Some("sésame")).unwrap();
            let mine = read(&me, None).unwrap();
            assert!(read(&eve, None).is_err());
            assert!(read(&bob, None).is_err(), "sans la phrase secrète de Bob");
            for d in [&for_bob, &mine] {
                // Une seule enveloppe : la signature est dans le message chiffré, le texte y est.
                let plain = String::from_utf8_lossy(&d.data);
                assert!(plain.contains("Secret") && !plain.contains("multipart/signed"));
            }
            assert!(for_bob
                .signature
                .is_some_and(|s| s.state == SigState::Valid));
        }

        #[test]
        fn une_phrase_fausse_ou_absente_est_signalee() {
            let key = keygen("Moi <me@x.fr>", Some("bonne"));
            for pw in [None, Some("mauvaise".to_string())] {
                let seal = Seal {
                    signer: Some((key.clone(), pw)),
                    recipients: vec![],
                };
                let err = build_sealed(&account(), &outgoing(vec![], None), &seal).unwrap_err();
                assert!(
                    err.to_wire().contains("pgp.passphrase_required"),
                    "{}",
                    err.to_wire()
                );
            }
            let seal = Seal {
                signer: Some((key, Some("bonne".into()))),
                recipients: vec![],
            };
            assert!(build_sealed(&account(), &outgoing(vec![], None), &seal).is_ok());
        }
    }
}
