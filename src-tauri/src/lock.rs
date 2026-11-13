//! Verrouillage de l'application : écran de déverrouillage au lancement et après inactivité.
//!
//! - **Système** (macOS : Touch ID ou mot de passe du Mac ; Windows : Windows Hello) : on délègue au système.
//! - **Mot de passe d'application** (Linux, ou si l'on préfère) : empreinte Argon2id gardée dans le trousseau du
//!   système, jamais en base ; essais ralentis après plusieurs échecs.
//!
//! La barrière est **dans le moteur** : tant que l'application est verrouillée, `lib.rs` refuse toute commande
//! hors celles de déverrouillage (voir `blocks`), quoi que fasse l'interface. Ce n'est pas un chiffrement :
//! quelqu'un qui lit directement les fichiers de l'application n'est pas arrêté (voir le chiffrement du disque par
//! le système), et le serveur MCP, qui lit la base sans passer par l'application, refuse de démarrer tant qu'un
//! verrouillage est actif.
//!
//! Si le déverrouillage devient impossible (mot de passe oublié, trousseau réinitialisé, Windows Hello retiré),
//! `reset_app` efface les données locales et le verrouillage : la contrepartie honnête d'un verrou sans chiffrement.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::error::{AppError, Result};
use crate::mail::account::{self, LOCAL_ID};

const SECRET_KIND: &str = "lock-password";
pub const MIN_PASSWORD_CHARS: usize = 8;
const MAX_PASSWORD_CHARS: usize = 256;
/// Échecs permis avant de ralentir ; ensuite 1 s, 2 s, 4 s… jusqu'à 5 minutes entre deux essais.
const FREE_ATTEMPTS: u32 = 4;
const MAX_DELAY: Duration = Duration::from_secs(300);

/// Commandes qui restent disponibles verrouillé : celles du déverrouillage, de la réinitialisation et de l'apparence.
const ALLOWED_WHEN_LOCKED: &[&str] = &[
    "app_info",
    "set_native_texts", // langue de l'invite de déverrouillage
    "system_appearance",
    "lock_status",
    "unlock_system",
    "unlock_password",
    "reset_app",
    "disk_encryption",
];
/// Plugins officiels de Tauri joignables verrouillé : fenêtre, notifications (sans contenu), événements, application
/// et boîtes de dialogue (confirmation de la réinitialisation). Ni `opener` (ouvrir des liens ou des dossiers), ni
/// aucun accès aux fichiers.
const ALLOWED_PLUGINS_WHEN_LOCKED: &[&str] = &[
    "plugin:window|",
    "plugin:notification|",
    "plugin:event|",
    "plugin:app|",
    "plugin:dialog|",
];

/// Une commande doit-elle être refusée ? (verrouillé, hors déverrouillage et plugins autorisés.)
pub fn blocks(locked: bool, command: &str) -> bool {
    locked
        && !ALLOWED_WHEN_LOCKED.contains(&command)
        && !ALLOWED_PLUGINS_WHEN_LOCKED
            .iter()
            .any(|p| command.starts_with(p))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    Off,
    System,
    Password,
}

impl Method {
    fn as_str(self) -> &'static str {
        match self {
            Method::Off => "off",
            Method::System => "system",
            Method::Password => "password",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub method: Method,
    /// Verrouillage automatique après cette durée d'inactivité (0 = jamais).
    pub timeout_secs: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            method: Method::Off,
            timeout_secs: 0,
        }
    }
}

/// Ce que l'interface a besoin de savoir.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub method: Method,
    pub locked: bool,
    pub timeout_secs: u32,
    /// Le système offre une authentification native (Touch ID, Windows Hello…) sur cette machine.
    pub system_available: bool,
}

pub const fn system_available() -> bool {
    cfg!(any(target_os = "macos", windows))
}

struct State {
    locked: bool,
    config: Config,
    failures: u32,
    not_before: Option<Instant>,
}

pub struct Lock {
    state: Mutex<State>,
    /// Un seul changement de réglage ou déverrouillage à la fois (une seule invite native, pas de course).
    busy: tokio::sync::Mutex<()>,
}

/// Verrouillée, ou verrou introuvable : on échoue fermé. Partagé par la barrière des commandes et par les
/// protocoles d'URI personnalisés (que la barrière des commandes ne couvre pas).
pub fn is_locked_or_missing<R: tauri::Runtime>(m: &impl tauri::Manager<R>) -> bool {
    m.try_state::<Lock>().map_or(true, |l| l.is_locked())
}

impl Lock {
    /// Au lancement, l'application est verrouillée dès qu'un verrouillage est configuré.
    pub fn new(config: Config) -> Self {
        Self {
            state: Mutex::new(State {
                locked: config.method != Method::Off,
                config,
                failures: 0,
                not_before: None,
            }),
            busy: tokio::sync::Mutex::new(()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_locked(&self) -> bool {
        self.state().locked
    }

    pub fn status(&self) -> Status {
        let s = self.state();
        Status {
            method: s.config.method,
            locked: s.locked,
            timeout_secs: s.config.timeout_secs,
            system_available: system_available(),
        }
    }

    pub fn lock_now(&self) -> Result<()> {
        let mut s = self.state();
        if s.config.method == Method::Off {
            return Err(AppError::Invalid(msg!(
                "lock.none_configured",
                "no lock is configured"
            )));
        }
        s.locked = true;
        Ok(())
    }

    /// Ralentissement après des échecs répétés : durée à attendre avant le prochain essai.
    fn wait_time(&self) -> Option<Duration> {
        self.state()
            .not_before?
            .checked_duration_since(Instant::now())
    }

    fn record(&self, ok: bool) {
        let mut s = self.state();
        if ok {
            s.failures = 0;
            s.not_before = None;
            return;
        }
        s.failures += 1;
        if s.failures > FREE_ATTEMPTS {
            let delay =
                crate::backoff::delay(1, MAX_DELAY.as_secs(), s.failures - FREE_ATTEMPTS - 1);
            s.not_before = Some(Instant::now() + delay);
        }
    }

    /// Authentifie l'utilisateur avec la méthode **active** : invite du système, ou mot de passe d'application
    /// (ralenti après des échecs). Ne change pas l'état de verrouillage.
    async fn authenticate(&self, method: Method, password: Option<String>) -> Result<()> {
        match method {
            Method::Off => Ok(()),
            Method::System => native_authenticate().await,
            Method::Password => {
                if let Some(wait) = self.wait_time() {
                    return Err(AppError::Auth(msg!(
                        "lock.too_many_attempts",
                        "too many attempts: try again in {seconds} s",
                        seconds = wait.as_secs() + 1
                    )));
                }
                let password = password.unwrap_or_default();
                // Argon2 est volontairement lent : hors du thread asynchrone.
                let ok = tokio::task::spawn_blocking(move || verify_password(&password))
                    .await
                    .map_err(|e| AppError::Auth((e.to_string()).into()))??;
                self.record(ok);
                ok.then_some(())
                    .ok_or_else(|| AppError::Auth(msg!("lock.wrong_password", "wrong password")))
            }
        }
    }

    /// Déverrouille : `method` doit être celle qui est configurée (le moteur ne se fie pas à l'interface).
    pub async fn unlock(&self, method: Method, password: Option<String>) -> Result<()> {
        let _one_at_a_time = self.busy.lock().await;
        if self.state().config.method != method {
            return Err(AppError::Invalid(msg!(
                "lock.method_inactive",
                "this lock method is not active"
            )));
        }
        self.authenticate(method, password).await?;
        self.state().locked = false;
        Ok(())
    }

    /// Change le réglage. Si un verrouillage est déjà actif, l'utilisateur doit s'authentifier **avant** (sinon
    /// quiconque s'assoit devant une session ouverte pourrait le retirer) : par le système, ou avec l'ancien mot de passe.
    pub async fn configure(
        &self,
        pool: &SqlitePool,
        next: Config,
        new_password: Option<String>,
        current_password: Option<String>,
    ) -> Result<()> {
        let _one_at_a_time = self.busy.lock().await;
        let current = self.state().config.method;
        self.authenticate(current, current_password).await?;
        match next.method {
            Method::Off | Method::System => {
                if next.method == Method::System && !system_available() {
                    return Err(AppError::Invalid(msg!(
                        "lock.no_native_choose_password",
                        "no native authentication on this system: choose a password"
                    )));
                }
                account::blocking_keychain(|| {
                    account::delete_app_secret(SECRET_KIND);
                    Ok(())
                })
                .await?;
            }
            Method::Password => {
                let password = new_password.ok_or_else(|| {
                    AppError::Invalid(msg!("lock.choose_password", "choose a password"))
                })?;
                let n = password.chars().count();
                if !(MIN_PASSWORD_CHARS..=MAX_PASSWORD_CHARS).contains(&n) {
                    return Err(AppError::Invalid(msg!(
                        "lock.password_length",
                        "the password must be {min} to {max} characters long",
                        min = MIN_PASSWORD_CHARS,
                        max = MAX_PASSWORD_CHARS
                    )));
                }
                // Argon2 (lent, volontairement) puis trousseau : un seul saut hors de l'exécuteur.
                account::blocking_keychain(move || {
                    account::store_app_secret(SECRET_KIND, &hash_password(&password)?)
                })
                .await?;
            }
        }
        save_config(pool, next).await?;
        let mut s = self.state();
        s.config = next;
        s.failures = 0;
        s.not_before = None;
        Ok(())
    }

    /// Remise à zéro après `reset_app` : plus de verrouillage, application ouverte. N'est possible que verrouillé.
    pub async fn reset(&self, pool: &SqlitePool) -> Result<()> {
        let _one_at_a_time = self.busy.lock().await;
        if !self.is_locked() {
            return Err(AppError::Invalid(msg!(
                "lock.not_locked",
                "the application is not locked"
            )));
        }
        wipe(pool).await?;
        account::blocking_keychain(|| {
            account::delete_app_secret(SECRET_KIND);
            Ok(())
        })
        .await?;
        let mut s = self.state();
        *s = State {
            locked: false,
            config: Config::default(),
            failures: 0,
            not_before: None,
        };
        Ok(())
    }
}

/// Efface toutes les données locales : comptes (et, en cascade, dossiers, messages, brouillons, programmés, alias),
/// carnet d'adresses, stockage local, modèles et réglages. Les e-mails restent sur les serveurs.
pub async fn wipe(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    for sql in [
        "DELETE FROM accounts WHERE id <> ?1",
        "DELETE FROM messages WHERE account_id = ?1",
        "DELETE FROM threads WHERE account_id = ?1",
        "DELETE FROM labels WHERE account_id = ?1",
        "DELETE FROM folders WHERE account_id = ?1",
        "DELETE FROM contacts WHERE ?1 IS NOT NULL",
        "DELETE FROM templates WHERE ?1 IS NOT NULL",
        "DELETE FROM settings WHERE ?1 IS NOT NULL",
    ] {
        sqlx::query(sql).bind(LOCAL_ID).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Paramètres Argon2id fixés (OWASP : 19 Mio, 2 passes, 1 voie) : une mise à jour de la bibliothèque ne les change pas.
fn argon2() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("paramètres Argon2 valides");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn hash_password(password: &str) -> Result<String> {
    argon2()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| {
            AppError::Auth(msg!(
                "lock.hash_failed",
                "password hashing failed: {detail}",
                detail = e
            ))
        })
}

fn verify_password(password: &str) -> Result<bool> {
    let stored = account::load_app_secret(SECRET_KIND)?
        .ok_or_else(|| AppError::Auth(msg!("lock.no_password_saved", "no password saved")))?;
    let parsed: PasswordHash = stored.parse().map_err(|e| {
        AppError::Auth(msg!(
            "lock.hash_unreadable",
            "unreadable password hash: {detail}",
            detail = e
        ))
    })?;
    Ok(argon2()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

// ───────────── Réglage persistant ─────────────

/// Lit le réglage. Une valeur inconnue (base abîmée) **échoue** plutôt que de valoir « désactivé » : un verrouillage
/// ne doit jamais s'ouvrir à cause d'une donnée corrompue.
pub async fn load_config(pool: &SqlitePool) -> Result<Config> {
    let get = |key: &'static str| async move {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
    };
    let method = match get("lock.method").await?.as_deref() {
        None | Some("off") => Method::Off,
        Some("system") => Method::System,
        Some("password") => Method::Password,
        Some(other) => {
            return Err(AppError::Invalid(msg!(
                "lock.setting_unreadable",
                "unreadable lock setting: “{value}”",
                value = other
            )))
        }
    };
    let timeout_secs = get("lock.timeout")
        .await?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Ok(Config {
        method,
        timeout_secs,
    })
}

async fn save_config(pool: &SqlitePool, c: Config) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (key, value) in [
        ("lock.method", c.method.as_str().to_string()),
        ("lock.timeout", c.timeout_secs.to_string()),
    ] {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

// ───────────── Authentification native ─────────────

/// Invite d'authentification du système. Tout se passe dans un fil dédié : le contexte natif n'a pas à traverser
/// les points d'attente asynchrones, et l'invite (Touch ID, Windows Hello) peut rester ouverte longtemps.
#[cfg(any(target_os = "macos", windows))]
async fn native_authenticate() -> Result<()> {
    use robius_authentication::{
        AndroidText, BiometricStrength, Context, PolicyBuilder, Text, WindowsText,
    };
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<()>>();
    std::thread::spawn(move || {
        let outcome = (|| {
            let policy = PolicyBuilder::new()
                .biometrics(Some(BiometricStrength::Strong))
                .password(true) // à défaut de biométrie : mot de passe de la session
                .companion(true)
                .build()
                .ok_or_else(|| {
                    AppError::Auth(msg!(
                        "lock.system_unavailable",
                        "system authentication unavailable"
                    ))
                })?;
            let texts = crate::native_text::get();
            let text = Text {
                android: AndroidText {
                    title: "Envelope",
                    subtitle: None,
                    description: None,
                },
                apple: &texts.unlock_reason,
                windows: WindowsText::new_truncated("Envelope", &texts.unlock_reason),
            };
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            Context::new(())
                .authenticate(text, &policy, move |r| {
                    let _ = done_tx.send(r.map_err(|e| AppError::Auth((e.to_string()).into())));
                })
                .map_err(|e| AppError::Auth((e.to_string()).into()))?;
            done_rx
                .recv_timeout(Duration::from_secs(180))
                .map_err(|_| AppError::Auth(msg!("lock.timeout", "timed out")))?
        })();
        let _ = tx.send(outcome);
    });
    rx.await
        .map_err(|_| AppError::Auth(msg!("lock.interrupted", "authentication interrupted")))?
}

#[cfg(not(any(target_os = "macos", windows)))]
async fn native_authenticate() -> Result<()> {
    Err(AppError::Auth(msg!(
        "lock.no_native",
        "no native authentication on this system"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password_lock() -> Lock {
        Lock::new(Config {
            method: Method::Password,
            timeout_secs: 0,
        })
    }

    #[test]
    fn commandes_refusees_verrouille() {
        assert!(blocks(true, "list_threads"));
        assert!(blocks(true, "send_email"));
        assert!(!blocks(true, "unlock_password"));
        assert!(!blocks(true, "lock_status"));
        assert!(!blocks(true, "reset_app"));
        assert!(!blocks(true, "plugin:dialog|ask"));
        assert!(!blocks(true, "plugin:notification|notify"));
        assert!(
            blocks(true, "plugin:opener|reveal_item_in_dir"),
            "ouvrir un dossier ou un lien : refusé"
        );
        assert!(
            blocks(true, "plugin"),
            "un nom qui ressemble seulement à un plugin n'est pas exempté"
        );
        assert!(!blocks(false, "list_threads"), "déverrouillé : tout passe");
    }

    #[test]
    fn verrouille_au_lancement_seulement_si_configure() {
        assert!(!Lock::new(Config::default()).is_locked());
        assert!(password_lock().is_locked());
        assert!(Lock::new(Config {
            method: Method::System,
            timeout_secs: 0
        })
        .is_locked());
    }

    #[test]
    fn verrouiller_exige_un_verrouillage_configure() {
        assert!(Lock::new(Config::default()).lock_now().is_err());
        let l = password_lock();
        l.state().locked = false;
        l.lock_now().unwrap();
        assert!(l.is_locked());
    }

    #[test]
    fn essais_ralentis_apres_des_echecs_puis_remis_a_zero() {
        let l = password_lock();
        for _ in 0..FREE_ATTEMPTS {
            l.record(false);
            assert!(
                l.wait_time().is_none(),
                "les premiers essais ne sont pas ralentis"
            );
        }
        l.record(false);
        let w1 = l.wait_time().expect("ralenti dès l'échec suivant");
        assert!(w1 <= Duration::from_secs(1));
        l.record(false);
        assert!(l.wait_time().unwrap() > w1, "le délai double");
        l.record(true);
        assert!(l.wait_time().is_none());
    }

    #[test]
    fn delai_plafonne() {
        let l = password_lock();
        for _ in 0..200 {
            l.record(false);
        }
        assert!(l.wait_time().unwrap() <= MAX_DELAY);
    }

    #[test]
    fn empreinte_argon2_aller_retour() {
        let h = hash_password("un mot de passe").unwrap();
        assert!(h.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{h}");
        let parsed: PasswordHash = h.parse().unwrap();
        assert!(argon2()
            .verify_password(b"un mot de passe", &parsed)
            .is_ok());
        assert!(argon2().verify_password(b"autre", &parsed).is_err());
    }

    #[tokio::test]
    async fn reglage_persistant_et_valeur_corrompue_refusee() {
        let pool = crate::db::init_memory().await.unwrap();
        assert_eq!(load_config(&pool).await.unwrap(), Config::default());
        let c = Config {
            method: Method::Password,
            timeout_secs: 900,
        };
        save_config(&pool, c).await.unwrap();
        assert_eq!(load_config(&pool).await.unwrap(), c);
        save_config(
            &pool,
            Config {
                method: Method::System,
                timeout_secs: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(load_config(&pool).await.unwrap().method, Method::System);
        sqlx::query("UPDATE settings SET value = 'n''importe quoi' WHERE key = 'lock.method'")
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            load_config(&pool).await.is_err(),
            "une valeur illisible n'ouvre pas le verrou"
        );
    }

    #[tokio::test]
    async fn deverrouillage_refuse_si_la_methode_ne_correspond_pas() {
        let l = Lock::new(Config {
            method: Method::System,
            timeout_secs: 0,
        });
        assert!(l.unlock(Method::Password, Some("x".into())).await.is_err());
        assert!(l.is_locked());
        let l = password_lock();
        assert!(l.unlock(Method::System, None).await.is_err());
        assert!(l.is_locked());
    }

    #[tokio::test]
    async fn reinitialisation_efface_tout_et_exige_d_etre_verrouille() {
        let pool = crate::db::init_memory().await.unwrap();
        sqlx::query("INSERT INTO accounts (id,email,username,imap_host,smtp_host) VALUES ('a','me@x.fr','a','i','s')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO templates (name, subject, body_html) VALUES ('t','s','b')")
            .execute(&pool)
            .await
            .unwrap();
        let c = Config {
            method: Method::Password,
            timeout_secs: 60,
        };
        save_config(&pool, c).await.unwrap();

        let lock = Lock::new(c);
        lock.state().locked = false;
        assert!(
            lock.reset(&pool).await.is_err(),
            "déverrouillé : pas de réinitialisation (on peut supprimer ses comptes normalement)"
        );

        lock.lock_now().unwrap();
        lock.reset(&pool).await.unwrap();
        assert!(!lock.is_locked());
        assert_eq!(lock.status().method, Method::Off);
        let comptes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE id <> 'local'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let carnet: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contacts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(carnet, 0);
        let modeles: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM templates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((comptes, modeles), (0, 0));
        assert_eq!(load_config(&pool).await.unwrap(), Config::default());
    }

    /// Parcours complet avec le **vrai trousseau du système** : `cargo test -p envelope lock::tests::parcours -- --ignored`.
    /// Ignoré par défaut (le trousseau peut demander une autorisation, et le test y écrit puis supprime l'entrée).
    #[tokio::test]
    #[ignore = "utilise le trousseau du système"]
    async fn parcours_avec_le_trousseau() {
        let pool = crate::db::init_memory().await.unwrap();
        let lock = Lock::new(Config::default());
        let on = Config {
            method: Method::Password,
            timeout_secs: 300,
        };

        lock.configure(&pool, on, Some("1234".into()), None)
            .await
            .expect_err("mot de passe trop court");
        lock.configure(&pool, on, Some("mot de passe 2026".into()), None)
            .await
            .unwrap();
        assert!(
            !lock.is_locked(),
            "on vient de le configurer : la session reste ouverte"
        );
        lock.lock_now().unwrap();
        assert!(lock.is_locked());

        assert!(lock
            .unlock(Method::Password, Some("mauvais".into()))
            .await
            .is_err());
        assert!(lock.is_locked());
        lock.unlock(Method::Password, Some("mot de passe 2026".into()))
            .await
            .unwrap();
        assert!(!lock.is_locked());

        // Retirer le verrouillage exige l'ancien mot de passe.
        lock.configure(&pool, Config::default(), None, Some("mauvais".into()))
            .await
            .expect_err("sans le bon mot de passe");
        assert_eq!(
            load_config(&pool).await.unwrap(),
            on,
            "le réglage n'a pas changé"
        );
        lock.configure(
            &pool,
            Config::default(),
            None,
            Some("mot de passe 2026".into()),
        )
        .await
        .unwrap();
        assert_eq!(load_config(&pool).await.unwrap(), Config::default());
        assert!(
            account::load_app_secret(SECRET_KIND).unwrap().is_none(),
            "l'empreinte est supprimée du trousseau"
        );
    }
}
