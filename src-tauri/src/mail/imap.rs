//! Connexion IMAP : TCP → TLS (rustls) → login (mot de passe) ou AUTHENTICATE XOAUTH2.
//! TLS implicite (port 993) ou STARTTLS (port 143, RFC 3501 §6.2.1). STARTTLS est **obligatoire**
//! une fois choisi : s'il est refusé, la connexion échoue (jamais de repli en clair, qui exposerait
//! le mot de passe). Une connexion sans chiffrement est refusée.

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::TlsConnector;

use async_imap::types::NameAttribute;
use futures::TryStreamExt;

use envelope_core::role::FolderRole;

use crate::error::{AppError, Result};
use crate::mail::account::{Account, Credential, Security};

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send + Debug {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Debug> Io for T {}

pub type ImapSession = async_imap::Session<Box<dyn Io>>;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// SASL XOAUTH2. Si le serveur répond par un défi (= erreur JSON), on renvoie une ligne vide.
struct XOAuth2 {
    payload: String,
    sent: bool,
}
impl async_imap::Authenticator for XOAuth2 {
    type Response = String;
    fn process(&mut self, _challenge: &[u8]) -> String {
        if self.sent {
            String::new()
        } else {
            self.sent = true;
            self.payload.clone()
        }
    }
}

pub async fn connect(account: &Account, cred: &Credential) -> Result<ImapSession> {
    connect_with(account, cred, crate::tls::client_config()?).await
}

/// `connect` avec une configuration TLS donnée (les tests y mettent leur propre autorité).
pub(crate) async fn connect_with(
    account: &Account,
    cred: &Credential,
    tls: Arc<ClientConfig>,
) -> Result<ImapSession> {
    let security = account.imap_security;
    if security == Security::None {
        return Err(AppError::Invalid(msg!(
            "imap.unencrypted",
            "IMAP: unencrypted connection refused (choose TLS or STARTTLS)"
        )));
    }
    let host = account.imap_host.as_str();
    let tcp = tokio::time::timeout(
        CONNECT_TIMEOUT,
        TcpStream::connect((host, account.imap_port as u16)),
    )
    .await
    .map_err(|_| {
        AppError::Invalid(msg!(
            "imap.timeout",
            "connection timed out: {host}",
            host = host
        ))
    })??;
    tcp.set_nodelay(true)?;
    let name = ServerName::try_from(host.to_string()).map_err(|e| {
        AppError::Invalid(msg!(
            "imap.invalid_host",
            "invalid host: {detail}",
            detail = e
        ))
    })?;
    let connector = TlsConnector::from(tls);
    let stream: Box<dyn Io> = match security {
        Security::Starttls => {
            // Échange en clair limité à la bannière et à STARTTLS ; le mot de passe ne part qu'après la mise en TLS.
            let mut plain = async_imap::Client::new(tcp);
            let _greeting = plain.read_response().await?; // bannière « * OK ... »
            plain
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|e| {
                    AppError::Auth(msg!(
                        "imap.starttls_refused",
                        "the server refuses STARTTLS: {detail}",
                        detail = e
                    ))
                })?;
            Box::new(connector.connect(name, plain.into_inner()).await?)
        }
        _ => Box::new(connector.connect(name, tcp).await?),
    };

    let mut client = async_imap::Client::new(stream);
    if security == Security::Tls {
        let _greeting = client.read_response().await?; // bannière « * OK ... »
    } // après STARTTLS, le serveur n'envoie pas de nouvelle bannière

    let session = match cred {
        Credential::Password { user, password } => client.login(user, password).await,
        Credential::OAuth { user, access_token } => {
            let payload = format!("user={user}\x01auth=Bearer {access_token}\x01\x01");
            client
                .authenticate(
                    "XOAUTH2",
                    XOAuth2 {
                        payload,
                        sent: false,
                    },
                )
                .await
        }
    };
    session.map_err(|(e, _client)| {
        AppError::Auth(msg!(
            "imap.refused",
            "IMAP connection refused: {detail}",
            detail = e
        ))
    })
}

/// Ensemble d'UID au format IMAP (`3,7,12`).
pub fn uid_set(uids: &[i64]) -> String {
    uids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Dernier segment d'un chemin de dossier (« INBOX.Sent » → « Sent »).
pub fn leaf<'a>(path: &'a str, delimiter: Option<&str>) -> &'a str {
    match delimiter {
        Some(d) if !d.is_empty() => path.rsplit(d).next().unwrap_or(path),
        _ => path,
    }
}

/// Rôle d'un dossier : attribut SPECIAL-USE du serveur, à défaut son nom (« Sent », « Envoyés »…).
pub fn folder_role(
    path: &str,
    delimiter: Option<&str>,
    attrs: &[NameAttribute],
) -> Option<FolderRole> {
    if path.eq_ignore_ascii_case("INBOX") {
        return Some(FolderRole::Inbox);
    }
    let by_attr = attrs.iter().find_map(|a| match a {
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Junk => Some(FolderRole::Junk),
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::All => Some(FolderRole::All),
        NameAttribute::Flagged => Some(FolderRole::Flagged),
        _ => None,
    });
    by_attr.or_else(|| match leaf(path, delimiter).to_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" | "envoyés" | "envoyes" | "éléments envoyés" => {
            Some(FolderRole::Sent)
        }
        "drafts" | "draft" | "brouillons" => Some(FolderRole::Drafts),
        "trash" | "deleted items" | "deleted messages" | "corbeille" | "éléments supprimés" => {
            Some(FolderRole::Trash)
        }
        "junk" | "spam" | "junk e-mail" | "courrier indésirable" | "indésirables" => {
            Some(FolderRole::Junk)
        }
        "archive" | "archives" => Some(FolderRole::Archive),
        _ => None,
    })
}

/// Chemin du dossier de ce rôle sur le serveur ; il est créé sous `default_name` (et abonné) s'il
/// n'existe pas encore, ce qui arrive sur une boîte neuve (ni « Envoyés » ni « Corbeille »).
pub async fn find_or_create(
    session: &mut ImapSession,
    role: FolderRole,
    default_name: &str,
) -> Result<String> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    let found = names.iter().find(|n| {
        !n.attributes()
            .iter()
            .any(|a| matches!(a, NameAttribute::NoSelect))
            && folder_role(n.name(), n.delimiter(), n.attributes()) == Some(role)
    });
    if let Some(n) = found {
        return Ok(n.name().to_string());
    }
    let path = new_folder_path(
        names.iter().map(|n| (n.name(), n.delimiter())),
        default_name,
    );
    // Boîte neuve (que INBOX) sur un serveur dont l'espace de noms personnel est `INBOX.` : rien à déduire de LIST,
    // la création à la racine est refusée, on réessaie sous INBOX (plutôt que de lire NAMESPACE, que ni async-imap ni
    // imap-proto n'exposent : un analyseur maison de réponses serveur serait une surface d'attaque).
    let path = match session.create(&path).await {
        Ok(()) => path,
        Err(first) => {
            let Some(under) = inbox_child(names.iter().map(|n| (n.name(), n.delimiter())), &path)
            else {
                return Err(first.into());
            };
            session.create(&under).await.map_err(|_| first)?;
            under
        }
    };
    let _ = session.subscribe(&path).await; // non bloquant : certains serveurs l'abonnent d'office
    Ok(path)
}

/// `INBOX<délimiteur>name` d'après le délimiteur annoncé pour INBOX (None s'il n'y en a pas ou si `path` est déjà dessous).
fn inbox_child<'a>(
    mut names: impl Iterator<Item = (&'a str, Option<&'a str>)>,
    path: &str,
) -> Option<String> {
    let delim = names
        .find(|(n, _)| n.eq_ignore_ascii_case("INBOX"))?
        .1
        .filter(|d| !d.is_empty())?;
    (!path.to_ascii_uppercase().starts_with("INBOX")).then(|| format!("INBOX{delim}{path}"))
}

/// Chemin sous lequel créer `name`. Un serveur dont l'espace de noms personnel est `INBOX.` (Courier, Dovecot en
/// configuration historique) refuse un dossier à la racine : si les dossiers existants sont tous des enfants d'INBOX,
/// on crée `INBOX<délimiteur>name`. Déduit de la réponse LIST déjà en main plutôt que de NAMESPACE, dont la réponse
/// demanderait un analyseur maison ; sur une boîte qui n'a encore que INBOX on retombe sur la racine.
fn new_folder_path<'a>(
    existing: impl Iterator<Item = (&'a str, Option<&'a str>)>,
    name: &str,
) -> String {
    let mut prefix = None;
    for (path, delimiter) in existing {
        if path.eq_ignore_ascii_case("INBOX") {
            continue;
        }
        let Some(d) = delimiter.filter(|d| !d.is_empty()) else {
            return name.to_string();
        };
        let under_inbox = path.len() > 5
            && path.is_char_boundary(5)
            && path[..5].eq_ignore_ascii_case("INBOX")
            && path[5..].starts_with(d);
        if !under_inbox {
            return name.to_string();
        }
        prefix = Some(format!("{}{d}", &path[..5]));
    }
    prefix.map_or_else(|| name.to_string(), |p| format!("{p}{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repli_sous_inbox_apres_refus_a_la_racine() {
        let l = |v: Vec<&'static str>| v.into_iter().map(|p| (p, Some(".")));
        assert_eq!(
            inbox_child(l(vec!["INBOX"]), "Sent").as_deref(),
            Some("INBOX.Sent")
        );
        assert_eq!(
            inbox_child(l(vec!["inbox"]), "Sent").as_deref(),
            Some("INBOX.Sent")
        );
        assert_eq!(inbox_child(l(vec!["INBOX"]), "INBOX.Sent"), None); // déjà dessous
        assert_eq!(inbox_child(l(vec!["Autre"]), "Sent"), None); // pas d'INBOX connue
        assert_eq!(inbox_child(std::iter::once(("INBOX", None)), "Sent"), None);
        // pas de délimiteur
    }

    #[test]
    fn dossier_cree_sous_inbox_si_le_serveur_l_exige() {
        let l = |v: Vec<&'static str>| v.into_iter().map(|p| (p, Some(".")));
        assert_eq!(
            new_folder_path(l(vec!["INBOX", "INBOX.Drafts", "INBOX.Trash"]), "Sent"),
            "INBOX.Sent"
        );
        assert_eq!(new_folder_path(l(vec!["INBOX", "Drafts"]), "Sent"), "Sent"); // racine permise
        assert_eq!(new_folder_path(l(vec!["INBOX"]), "Sent"), "Sent"); // rien à déduire
        assert_eq!(
            new_folder_path(l(vec!["INBOX", "INBOX.A", "Autre"]), "Sent"),
            "Sent"
        );
        assert_eq!(
            new_folder_path(std::iter::once(("INBOX/x", Some("/"))), "Sent"),
            "INBOX/Sent"
        );
    }

    #[test]
    fn roles_from_attribute_or_name() {
        assert_eq!(folder_role("inbox", None, &[]), Some(FolderRole::Inbox));
        assert_eq!(
            folder_role("Whatever", Some("/"), &[NameAttribute::Sent]),
            Some(FolderRole::Sent)
        );
        assert_eq!(
            folder_role("INBOX.Éléments envoyés", Some("."), &[]),
            Some(FolderRole::Sent)
        );
        assert_eq!(folder_role("Sent", Some("/"), &[]), Some(FolderRole::Sent));
        assert_eq!(
            folder_role("Brouillons", None, &[]),
            Some(FolderRole::Drafts)
        );
        assert_eq!(folder_role("Projets/Sent-2024", Some("/"), &[]), None);
    }
}

#[cfg(test)]
pub(crate) mod starttls_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio_rustls::rustls::RootCertStore;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::TlsAcceptor;

    fn account(port: u16, security: Security) -> Account {
        Account {
            display_name: None,
            imap_host: "localhost".into(),
            imap_port: i64::from(port),
            imap_security: security,
            ..Account::fixture()
        }
    }

    fn cred() -> Credential {
        Credential::Password {
            user: "me".into(),
            password: "secret".into(),
        }
    }

    /// Autorité de test : certificat auto-signé pour `localhost`, côté serveur et côté client.
    pub(crate) fn tls_pair() -> (TlsAcceptor, Arc<ClientConfig>) {
        let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
        let ck = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let cert: CertificateDer<'static> = ck.cert.der().clone();
        let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(ck.signing_key.serialize_der()));
        let server = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert.clone()], key)
            .unwrap();
        let mut roots = RootCertStore::empty();
        roots.add(cert).unwrap();
        let client = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        (TlsAcceptor::from(Arc::new(server)), Arc::new(client))
    }

    /// Serveur IMAP minimal : bannière, STARTTLS (accepté ou refusé), puis LOGIN. Retourne ce qu'il a lu.
    async fn mock_server(
        accept_starttls: bool,
        acceptor: TlsAcceptor,
    ) -> (u16, tokio::task::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut seen = Vec::new();
            let mut reader = BufReader::new(tcp);
            reader
                .get_mut()
                .write_all(b"* OK IMAP4rev1 ready\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            seen.push(format!("clair: {}", line.trim()));
            let tag = line.split_whitespace().next().unwrap_or("A").to_string();
            if !accept_starttls {
                reader
                    .get_mut()
                    .write_all(format!("{tag} NO STARTTLS non disponible\r\n").as_bytes())
                    .await
                    .unwrap();
                // Si le client insistait en clair, on le verrait ici.
                let mut rest = String::new();
                let _ = tokio::time::timeout(
                    std::time::Duration::from_millis(300),
                    reader.read_line(&mut rest),
                )
                .await;
                if !rest.is_empty() {
                    seen.push(format!("clair: {}", rest.trim()));
                }
                return seen;
            }
            reader
                .get_mut()
                .write_all(format!("{tag} OK Begin TLS negotiation now\r\n").as_bytes())
                .await
                .unwrap();
            // Un client qui refuse le certificat coupe la négociation : rien de plus n'est lu.
            let Ok(tls) = acceptor.accept(reader.into_inner()).await else {
                return seen;
            };
            let mut reader = BufReader::new(tls);
            let mut login = String::new();
            reader.read_line(&mut login).await.unwrap();
            seen.push(format!("tls: {}", login.trim()));
            let tag = login.split_whitespace().next().unwrap_or("A").to_string();
            reader
                .get_mut()
                .write_all(format!("{tag} OK LOGIN completed\r\n").as_bytes())
                .await
                .unwrap();
            seen
        });
        (port, task)
    }

    #[tokio::test]
    async fn starttls_negocie_puis_authentifie_sous_tls() {
        let (acceptor, client_cfg) = tls_pair();
        let (port, server) = mock_server(true, acceptor).await;
        connect_with(&account(port, Security::Starttls), &cred(), client_cfg)
            .await
            .expect("connexion STARTTLS");
        let seen = server.await.unwrap();
        // Seul STARTTLS circule en clair ; le mot de passe n'est envoyé qu'une fois la session chiffrée.
        assert!(
            seen[0].starts_with("clair:") && seen[0].contains("STARTTLS"),
            "{seen:?}"
        );
        assert!(
            seen[1].starts_with("tls:") && seen[1].contains("LOGIN"),
            "{seen:?}"
        );
        assert!(!seen[0].contains("secret"));
    }

    #[tokio::test]
    async fn starttls_refuse_pas_de_repli_en_clair() {
        let (acceptor, client_cfg) = tls_pair();
        let (port, server) = mock_server(false, acceptor).await;
        let err = connect_with(&account(port, Security::Starttls), &cred(), client_cfg)
            .await
            .expect_err("doit échouer")
            .to_string();
        assert!(err.contains("STARTTLS"), "{err}");
        let seen = server.await.unwrap();
        assert_eq!(
            seen.len(),
            1,
            "le client n'a pas envoyé son mot de passe en clair : {seen:?}"
        );
    }

    /// Avec la configuration par défaut (vérification par le système), un certificat auto-signé est
    /// refusé et le mot de passe n'est jamais envoyé.
    #[tokio::test]
    async fn certificat_inconnu_refuse_et_mot_de_passe_protege() {
        let (acceptor, _own_roots) = tls_pair();
        let (port, server) = mock_server(true, acceptor).await;
        let err = connect(&account(port, Security::Starttls), &cred())
            .await
            .expect_err("certificat inconnu")
            .to_string();
        assert!(!err.is_empty());
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 1, "seul STARTTLS a circulé : {seen:?}");
        assert!(seen.iter().all(|l| !l.contains("secret")));
    }

    #[tokio::test]
    async fn sans_chiffrement_refuse_sans_se_connecter() {
        let (_, client_cfg) = tls_pair();
        // Aucun serveur n'écoute : l'erreur vient de la politique, pas du réseau.
        let err = connect_with(&account(1, Security::None), &cred(), client_cfg)
            .await
            .expect_err("doit échouer");
        assert!(
            matches!(&err, AppError::Invalid(m) if m.code == "imap.unencrypted"),
            "{err}"
        );
    }
}
