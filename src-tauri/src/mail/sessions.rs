//! Sessions IMAP réutilisées entre les actions de l'utilisateur (archiver, lire, étiqueter, supprimer,
//! brouillons, archivage local). Une action isolée coûtait TCP + TLS + authentification (+ trousseau ou
//! renouvellement du jeton OAuth) ; enchaînées, elles réutilisent maintenant la session de la précédente.
//!
//! La synchronisation garde sa propre session (IDLE) et n'en dépend pas.
//!
//! Règles :
//! - Une session inactive par compte, gardée au plus `MAX_IDLE` : un serveur plafonne les connexions
//!   simultanées (Gmail : 15), on ne les occupe pas pour rien.
//! - Avant réemploi, un `NOOP` : une session coupée par le serveur ou le réseau est remplacée sans que
//!   l'action échoue.
//! - Le bail ne rend sa session qu'avec `release()` (opération menée à son terme) ; tout autre sortie (erreur,
//!   `?`, annulation) la ferme : une session dans un état douteux ne repart jamais.
//! - Hôte, port, sécurité ou identifiant différents de ceux de la session gardée : elle n'est pas réutilisée.
//!   Les secrets modifiés ou supprimés passent par `forget`.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use sqlx::SqlitePool;

use crate::error::Result;
use crate::mail::account::{self, Account, Credential};
use crate::mail::imap::{self, ImapSession};

const MAX_IDLE: Duration = Duration::from_secs(60);
/// Un serveur qui ne répond pas au `NOOP` ou à `LOGOUT` ne retient personne.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

struct Idle {
    session: ImapSession,
    /// Ce avec quoi elle a été ouverte (voir `fingerprint`).
    fingerprint: String,
    /// Minuteur qui la ferme au bout de `MAX_IDLE` ; arrêté dès qu'elle quitte la réserve.
    expiry: tokio::task::AbortHandle,
}

impl Idle {
    /// Sort de la réserve : le minuteur n'a plus lieu d'être.
    fn into_session(self) -> ImapSession {
        self.expiry.abort();
        self.session
    }
}

static IDLE: LazyLock<Mutex<HashMap<String, Idle>>> = LazyLock::new(Mutex::default);

/// Incrémentée par `forget` : un bail ouvert avant ne rend pas sa session (elle date des anciens secrets).
static EPOCH: AtomicU64 = AtomicU64::new(0);

/// Verrou court, jamais tenu à travers un `.await`. Un verrou empoisonné (panique ailleurs) ne bloque pas les actions.
fn slots() -> MutexGuard<'static, HashMap<String, Idle>> {
    IDLE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fingerprint(account: &Account) -> String {
    format!(
        "{}:{}:{:?}:{}",
        account.imap_host, account.imap_port, account.imap_security, account.username
    )
}

/// Session prêtée : se manipule comme une `ImapSession`. À la fin d'une opération réussie, `release()`
/// la remet en réserve ; sinon elle est fermée.
pub struct Lease {
    account_id: String,
    fingerprint: String,
    epoch: u64,
    session: Option<ImapSession>,
}

impl Deref for Lease {
    type Target = ImapSession;
    fn deref(&self) -> &ImapSession {
        self.session
            .as_ref()
            .expect("session présente jusqu'à release")
    }
}

impl DerefMut for Lease {
    fn deref_mut(&mut self) -> &mut ImapSession {
        self.session
            .as_mut()
            .expect("session présente jusqu'à release")
    }
}

impl Lease {
    /// Remet la session en réserve (la précédente du compte, s'il y en a une, est fermée).
    pub fn release(mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if self.epoch != EPOCH.load(Ordering::SeqCst) {
            return close(session); // les secrets ont changé pendant l'opération
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return close(session);
        };
        let account_id = std::mem::take(&mut self.account_id);
        let key = account_id.clone();
        let expiry = runtime
            .spawn(async move {
                tokio::time::sleep(MAX_IDLE).await;
                let expired = slots().remove(&account_id);
                if let Some(idle) = expired {
                    close(idle.session);
                }
            })
            .abort_handle();
        let idle = Idle {
            session,
            fingerprint: std::mem::take(&mut self.fingerprint),
            expiry,
        };
        if let Some(previous) = slots().insert(key, idle) {
            close(previous.into_session());
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            close(session);
        }
    }
}

/// Ferme proprement (`LOGOUT`) sans faire attendre l'appelant.
fn close(mut session: ImapSession) {
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            let _ = tokio::time::timeout(PROBE_TIMEOUT, session.logout()).await;
        });
    }
}

/// Session du compte : celle de la réserve si elle répond encore, sinon une nouvelle connexion
/// (les identifiants ne sont lus — trousseau, jeton OAuth — que dans ce cas).
pub async fn acquire(sql: &SqlitePool, account_id: &str) -> Result<Lease> {
    let acct = account::get(sql, account_id).await?;
    if let Some(lease) = reuse(&acct).await {
        return Ok(lease);
    }
    let cred = account::resolve_credential(&acct).await?;
    connect(&acct, &cred).await
}

/// `acquire` pour un appelant qui a déjà ses identifiants.
pub async fn acquire_with(account: &Account, cred: &Credential) -> Result<Lease> {
    match reuse(account).await {
        Some(lease) => Ok(lease),
        None => connect(account, cred).await,
    }
}

async fn connect(account: &Account, cred: &Credential) -> Result<Lease> {
    let epoch = EPOCH.load(Ordering::SeqCst);
    let session = imap::connect(account, cred).await?;
    Ok(Lease {
        account_id: account.id.clone(),
        fingerprint: fingerprint(account),
        epoch,
        session: Some(session),
    })
}

async fn reuse(account: &Account) -> Option<Lease> {
    let epoch = EPOCH.load(Ordering::SeqCst);
    let idle = {
        let mut slots = slots();
        match slots.remove(&account.id) {
            Some(i) if i.fingerprint == fingerprint(account) => i,
            Some(stale) => {
                close(stale.into_session());
                return None;
            }
            None => return None,
        }
    };
    let fingerprint = idle.fingerprint.clone();
    let mut session = idle.into_session();
    match tokio::time::timeout(PROBE_TIMEOUT, session.noop()).await {
        Ok(Ok(())) => Some(Lease {
            account_id: account.id.clone(),
            fingerprint,
            epoch,
            session: Some(session),
        }),
        _ => None, // coupée, ou muette : on la laisse tomber, l'appelant se reconnecte
    }
}

/// Ferme la session gardée du compte : ses secrets ont changé ou ont été supprimés. Les baux en cours
/// ne la remettront pas en réserve.
pub fn forget(account_id: &str) {
    EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Some(idle) = slots().remove(account_id) {
        close(idle.into_session());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::account::Security;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio_rustls::rustls::ClientConfig;
    use tokio_rustls::TlsAcceptor;

    /// Serveur IMAP minimal en TLS implicite : bannière, LOGIN, NOOP, LOGOUT. Compte les connexions et les NOOP ;
    /// `drop_after_login` ferme la connexion juste après l'authentification (session « coupée par le serveur »).
    struct Server {
        port: u16,
        connections: Arc<AtomicUsize>,
        noops: Arc<AtomicUsize>,
        client: Arc<ClientConfig>,
    }

    async fn serve(drop_after_login: bool) -> Server {
        let (acceptor, client) = crate::mail::imap::starttls_tests::tls_pair();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicUsize::new(0));
        let noops = Arc::new(AtomicUsize::new(0));
        let (c, n) = (connections.clone(), noops.clone());
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                c.fetch_add(1, Ordering::SeqCst);
                let acceptor = acceptor.clone();
                let n = n.clone();
                tokio::spawn(async move { session(tcp, acceptor, n, drop_after_login).await });
            }
        });
        Server {
            port,
            connections,
            noops,
            client,
        }
    }

    async fn session(
        tcp: TcpStream,
        acceptor: TlsAcceptor,
        noops: Arc<AtomicUsize>,
        drop_after_login: bool,
    ) {
        let Ok(tls) = acceptor.accept(tcp).await else {
            return;
        };
        let mut reader = BufReader::new(tls);
        if reader.get_mut().write_all(b"* OK ready\r\n").await.is_err() {
            return;
        }
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let mut words = line.split_whitespace();
            let (tag, command) = (
                words.next().unwrap_or("A").to_string(),
                words.next().unwrap_or("").to_uppercase(),
            );
            let reply = match command.as_str() {
                "NOOP" => {
                    noops.fetch_add(1, Ordering::SeqCst);
                    format!("{tag} OK NOOP completed\r\n")
                }
                "LOGOUT" => {
                    let _ = reader
                        .get_mut()
                        .write_all(format!("* BYE\r\n{tag} OK bye\r\n").as_bytes())
                        .await;
                    return;
                }
                _ => format!("{tag} OK {command} completed\r\n"),
            };
            if reader.get_mut().write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            if drop_after_login && command == "LOGIN" {
                return; // le serveur coupe la session aussitôt authentifiée
            }
        }
    }

    fn account(port: u16) -> Account {
        Account {
            imap_host: "localhost".into(),
            imap_port: i64::from(port),
            imap_security: Security::Tls,
            ..Account::fixture()
        }
    }

    fn cred() -> Credential {
        Credential::Password {
            user: "me".into(),
            password: "secret".into(),
        }
    }

    /// `acquire_with` passe par la configuration TLS du système, qui refuse l'autorité de test : le banc ouvre donc
    /// la session lui-même, comme `connect`, avec la configuration de test.
    async fn open(server: &Server, acct: &Account) -> Lease {
        let session = imap::connect_with(acct, &cred(), server.client.clone())
            .await
            .unwrap();
        Lease {
            account_id: acct.id.clone(),
            fingerprint: fingerprint(acct),
            epoch: EPOCH.load(Ordering::SeqCst),
            session: Some(session),
        }
    }

    #[tokio::test]
    async fn session_rendue_est_reutilisee_apres_un_noop() {
        let server = serve(false).await;
        let acct = Account {
            id: "reuse".into(),
            ..account(server.port)
        };
        open(&server, &acct).await.release();
        let lease = reuse(&acct).await.expect("réutilisée");
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            1,
            "aucune nouvelle connexion"
        );
        assert_eq!(
            server.noops.load(Ordering::SeqCst),
            1,
            "vérifiée par un NOOP"
        );
        drop(lease);
        assert!(
            reuse(&acct).await.is_none(),
            "un bail abandonné sans release() n'est pas gardé"
        );
    }

    #[tokio::test]
    async fn session_coupee_par_le_serveur_n_est_pas_reutilisee() {
        let server = serve(true).await;
        let acct = Account {
            id: "cut".into(),
            ..account(server.port)
        };
        open(&server, &acct).await.release();
        assert!(
            reuse(&acct).await.is_none(),
            "le NOOP échoue : on se reconnectera"
        );
    }

    #[tokio::test]
    async fn autre_serveur_ou_identifiant_ne_reutilise_pas() {
        let server = serve(false).await;
        let acct = Account {
            id: "fp".into(),
            ..account(server.port)
        };
        open(&server, &acct).await.release();
        let moved = Account {
            imap_host: "autre".into(),
            ..acct.clone()
        };
        assert!(reuse(&moved).await.is_none());
        assert_eq!(
            server.noops.load(Ordering::SeqCst),
            0,
            "écartée sans même la sonder"
        );
    }

    #[tokio::test]
    async fn forget_ferme_la_session_gardee() {
        let server = serve(false).await;
        let acct = Account {
            id: "forget".into(),
            ..account(server.port)
        };
        open(&server, &acct).await.release();
        forget(&acct.id);
        assert!(reuse(&acct).await.is_none());
    }

    #[tokio::test]
    async fn bail_ouvert_avant_forget_ne_rend_pas_sa_session() {
        let server = serve(false).await;
        let acct = Account {
            id: "race".into(),
            ..account(server.port)
        };
        let lease = open(&server, &acct).await;
        forget(&acct.id); // les secrets changent pendant que l'opération tourne
        lease.release();
        assert!(reuse(&acct).await.is_none());
    }

    #[tokio::test]
    async fn session_inactive_expire() {
        let server = serve(false).await;
        let acct = Account {
            id: "expire".into(),
            ..account(server.port)
        };
        let lease = open(&server, &acct).await;
        tokio::time::pause(); // après la connexion réelle : seul le minuteur d'expiration est simulé
        lease.release();
        tokio::time::sleep(MAX_IDLE + Duration::from_secs(1)).await;
        assert!(!slots().contains_key("expire"));
    }
}
