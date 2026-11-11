//! Daemon de synchronisation : une tâche tokio par compte.
//!
//! Cycle : connexion → LIST des dossiers → passe complète (tous les dossiers) → IDLE sur INBOX.
//! À chaque réveil (nouveau message, timeout 10 min, ou `trigger`) on resynchronise INBOX ;
//! une passe complète est refaite si la dernière date de > 10 min ou sur demande manuelle.
//! En cas d'erreur : reconnexion avec backoff exponentiel (5 s → 5 min).
//!
//! Par dossier : UIDVALIDITY (purge si changé) ; réconciliation flags/suppressions sur la fenêtre
//! locale ; fetch incrémental des UID > max local (fenêtre initiale : 200 derniers messages).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_imap::extensions::idle::IdleResponse;
use async_imap::types::{Fetch, Flag, NameAttribute};
use base64::{
    alphabet, engine::general_purpose::GeneralPurpose,
    engine::general_purpose::GeneralPurposeConfig, engine::DecodePaddingMode, Engine,
};
use futures::{FutureExt, TryStreamExt};
use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};
use tauri::async_runtime::JoinHandle;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

use crate::error::{AppError, Result};
use crate::mail::imap::{self, ImapSession};
use crate::mail::{account, ingest, rawstore};

const INITIAL_WINDOW: i64 = 200;
/// Lot de l'historique rapatrié (messages par requête IMAP) et durée maximale d'une série de lots
/// avant de rendre la main à la synchro courante (nouveaux messages, demandes manuelles).
const BACKFILL_BATCH: i64 = 100;
const BACKFILL_BUDGET: Duration = Duration::from_secs(15);
/// Hors passe complète, nombre de messages récents par dossier dont on réaligne les drapeaux.
const RECONCILE_RECENT: i64 = 2000;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const FULL_SYNC_EVERY: Duration = Duration::from_secs(10 * 60);
const MIN_BACKOFF: Duration = Duration::from_secs(5);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct Ctx {
    pool: SqlitePool,
    app: AppHandle,
    attachments_dir: PathBuf,
    raw_dir: PathBuf,
    account_id: String,
    notify: Arc<Notify>,
}

struct Task {
    join: JoinHandle<()>,
    notify: Arc<Notify>,
}

pub struct SyncManager {
    pool: SqlitePool,
    app: AppHandle,
    attachments_dir: PathBuf,
    raw_dir: PathBuf,
    tasks: Mutex<HashMap<String, Task>>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SyncedEvent {
    account_id: String,
    new_messages: usize,
    /// Nouveaux messages non lus de la boîte de réception (de quoi notifier), plafonnés.
    fresh: Vec<FreshMail>,
    /// Nombre total de ces messages (au-delà du plafond de `fresh`).
    fresh_total: usize,
}

/// Aperçu d'un nouveau message pour la notification. Contenu de l'expéditeur : non fiable, affiché en texte brut.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct FreshMail {
    from: String,
    subject: String,
}

/// Nombre maximal d'aperçus joints à un événement de synchro.
const MAX_FRESH: usize = 3;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct StatusEvent {
    account_id: String,
    status: &'static str,
    error: Option<String>,
}

impl SyncManager {
    pub fn new(
        pool: SqlitePool,
        app: AppHandle,
        attachments_dir: PathBuf,
        raw_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            app,
            attachments_dir,
            raw_dir,
            tasks: Mutex::new(HashMap::new()),
        })
    }

    pub async fn start_all(self: &Arc<Self>) -> Result<()> {
        for a in account::list(&self.pool).await? {
            self.start(a.id);
        }
        Ok(())
    }

    /// Démarre (ou redémarre) la synchro d'un compte.
    pub fn start(self: &Arc<Self>, account_id: String) {
        self.stop(&account_id);
        let notify = Arc::new(Notify::new());
        let ctx = Ctx {
            pool: self.pool.clone(),
            app: self.app.clone(),
            attachments_dir: self.attachments_dir.clone(),
            raw_dir: self.raw_dir.clone(),
            account_id: account_id.clone(),
            notify: notify.clone(),
        };
        let join = tauri::async_runtime::spawn(run_account(ctx));
        self.tasks
            .lock()
            .unwrap()
            .insert(account_id, Task { join, notify });
    }

    pub fn stop(&self, account_id: &str) {
        if let Some(t) = self.tasks.lock().unwrap().remove(account_id) {
            t.join.abort();
        }
    }

    /// Demande une synchro complète immédiate (interrompt l'IDLE ou le backoff en cours).
    pub fn trigger(&self, account_id: &str) {
        if let Some(t) = self.tasks.lock().unwrap().get(account_id) {
            t.notify.notify_one();
        }
    }
}

impl Ctx {
    async fn set_status(&self, status: &'static str, error: Option<String>) {
        let _ = sqlx::query(
            "UPDATE accounts SET sync_status = ?1, sync_error = ?2, \
             last_sync_at = CASE WHEN ?1 = 'idle' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE last_sync_at END \
             WHERE id = ?3",
        )
        .bind(status).bind(&error).bind(&self.account_id).execute(&self.pool).await;
        let _ = self.app.emit(
            "mail:status",
            StatusEvent {
                account_id: self.account_id.clone(),
                status,
                error,
            },
        );
    }
}

async fn run_account(ctx: Ctx) {
    let mut backoff = MIN_BACKOFF;
    loop {
        match session_loop(&ctx, &mut backoff).await {
            Err(AppError::NotFound(_)) => return, // compte supprimé
            Err(e) => {
                tracing::warn!(account = %ctx.account_id, "synchro interrompue : {e}");
                ctx.set_status("error", Some(e.to_string())).await;
            }
            Ok(()) => {}
        }
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = ctx.notify.notified() => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

async fn session_loop(ctx: &Ctx, backoff: &mut Duration) -> Result<()> {
    ctx.set_status("syncing", None).await;
    let acct = account::get(&ctx.pool, &ctx.account_id).await?;
    let cred = account::resolve_credential(&acct).await?;
    let mut session = imap::connect(&acct, &cred).await?;
    *backoff = MIN_BACKOFF; // connexion OK
    let idle_supported = session.capabilities().await?.has_str("IDLE");

    let mut full = true;
    let mut last_full = Instant::now();
    let mut folders = Vec::new();
    loop {
        ctx.set_status("syncing", None).await;
        // LIST + upsert des dossiers : seulement sur passe complète (un réveil IDLE ne change pas l'arbre).
        if full || folders.is_empty() {
            folders = sync_folder_list(&mut session, ctx).await?;
        }
        let mut touched = HashSet::new();
        let mut new_messages = 0;
        let mut fresh = Vec::new();
        for f in folders
            .iter()
            .filter(|f| full || f.role.as_deref() == Some("inbox"))
        {
            new_messages +=
                sync_folder(&mut session, ctx, f, &mut touched, &mut fresh, full).await?;
        }
        ingest::refresh_threads(&ctx.pool, &touched).await?;
        if full {
            last_full = Instant::now();
        }
        ctx.set_status("idle", None).await;
        let _ = ctx.app.emit(
            "mail:synced",
            SyncedEvent {
                account_id: ctx.account_id.clone(),
                new_messages,
                fresh_total: fresh.len(),
                fresh: fresh.into_iter().take(MAX_FRESH).collect(),
            },
        );

        // Historique : un ou plusieurs lots, puis on repasse par la synchro courante (pas d'attente).
        let more = backfill(&mut session, ctx, &folders).await?;
        if more {
            let forced = ctx.notify.notified().now_or_never().is_some();
            full = forced || last_full.elapsed() >= FULL_SYNC_EVERY;
            continue;
        }

        let inbox = folders
            .iter()
            .find(|f| f.role.as_deref() == Some("inbox"))
            .map(|f| f.path.clone())
            .unwrap_or_else(|| "INBOX".into());
        let forced;
        (session, forced) = if idle_supported {
            idle_wait(session, &inbox, &ctx.notify).await?
        } else {
            polling_wait(session, &ctx.notify).await?
        };
        full = forced || last_full.elapsed() >= FULL_SYNC_EVERY;
    }
}

/// Retourne `true` si une passe complète est nécessaire (timeout ou demande manuelle).
async fn idle_wait(
    mut session: ImapSession,
    inbox: &str,
    notify: &Notify,
) -> Result<(ImapSession, bool)> {
    session.select(inbox).await?;
    let mut handle = session.idle();
    handle.init().await?;
    let forced = {
        let (fut, stop) = handle.wait_with_timeout(IDLE_TIMEOUT);
        let mut fut = Box::pin(fut);
        tokio::select! {
            r = &mut fut => !matches!(r?, IdleResponse::NewData(_)),
            _ = notify.notified() => {
                drop(stop); // interrompt l'IDLE proprement
                let _ = fut.await;
                true
            }
        }
    };
    Ok((handle.done().await?, forced))
}

/// Serveurs sans IDLE : NOOP toutes les 60 s.
async fn polling_wait(mut session: ImapSession, notify: &Notify) -> Result<(ImapSession, bool)> {
    let forced = tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(60)) => false,
        _ = notify.notified() => true,
    };
    session.noop().await?;
    Ok((session, forced))
}

#[derive(Debug, sqlx::FromRow)]
struct FolderRec {
    id: i64,
    path: String,
    role: Option<String>,
    uid_validity: Option<i64>,
}

/// Décode l'UTF-7 modifié des noms de dossiers IMAP (RFC 3501 §5.1.3) : `Envoy&AOk-s` → `Envoyés`.
pub fn decode_mutf7(s: &str) -> String {
    let engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let b64: String = it.by_ref().take_while(|&c| c != '-').collect();
        if b64.is_empty() {
            out.push('&');
            continue;
        }
        match engine.decode(b64.replace(',', "/")) {
            Ok(bytes) => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|p| u16::from_be_bytes([p[0], p[1]]))
                    .collect();
                out.push_str(&String::from_utf16_lossy(&units));
            }
            Err(_) => {
                out.push('&');
                out.push_str(&b64);
                out.push('-');
            }
        }
    }
    out
}

async fn sync_folder_list(session: &mut ImapSession, ctx: &Ctx) -> Result<Vec<FolderRec>> {
    let names: Vec<_> = session
        .list(Some(""), Some("*"))
        .await?
        .try_collect()
        .await?;
    let mut paths = Vec::new();
    for n in &names {
        if n.attributes()
            .iter()
            .any(|a| matches!(a, NameAttribute::NoSelect))
        {
            continue; // conteneur de hiérarchie (ex. « [Gmail] »)
        }
        let path = n.name().to_string();
        let role = imap::folder_role(&path, n.delimiter(), n.attributes());
        let delim = n.delimiter();
        let leaf = imap::leaf(&path, delim);
        sqlx::query(
            "INSERT INTO folders (account_id, path, name, role, delimiter) VALUES (?,?,?,?,?) \
             ON CONFLICT(account_id, path) DO UPDATE SET name = excluded.name, role = excluded.role, delimiter = excluded.delimiter",
        )
        .bind(&ctx.account_id)
        .bind(&path)
        .bind(decode_mutf7(leaf))
        .bind(role)
        .bind(delim)
        .execute(&ctx.pool)
        .await?;
        paths.push(path);
    }
    if !paths.is_empty() {
        // Dossiers supprimés côté serveur (les messages partent en cascade).
        let mut qb: QueryBuilder<Sqlite> =
            QueryBuilder::new("DELETE FROM folders WHERE account_id = ");
        qb.push_bind(&ctx.account_id).push(" AND path NOT IN (");
        let mut sep = qb.separated(", ");
        for p in &paths {
            sep.push_bind(p);
        }
        qb.push(")");
        qb.build().execute(&ctx.pool).await?;
    }
    // « Tous les messages » (Gmail) dupliquerait tout : non synchronisé.
    Ok(sqlx::query_as(
        "SELECT id, path, role, uid_validity FROM folders WHERE account_id = ? AND COALESCE(role,'') <> 'all' \
         ORDER BY (role = 'inbox') DESC, path",
    )
    .bind(&ctx.account_id)
    .fetch_all(&ctx.pool)
    .await?)
}

/// Synchronise un dossier ; retourne le nombre de nouveaux messages.
async fn sync_folder(
    session: &mut ImapSession,
    ctx: &Ctx,
    f: &FolderRec,
    touched: &mut HashSet<i64>,
    fresh: &mut Vec<FreshMail>,
    full: bool,
) -> Result<usize> {
    let mbox = session.select(&f.path).await?;
    let uid_validity = mbox.uid_validity.map(i64::from);
    let uid_next = mbox.uid_next.map(i64::from);

    if f.uid_validity.is_some() && f.uid_validity != uid_validity {
        tracing::info!(folder = %f.path, "UIDVALIDITY changé : purge du cache local");
        let tids: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT thread_id FROM messages WHERE folder_id = ? AND thread_id IS NOT NULL",
        )
        .bind(f.id)
        .fetch_all(&ctx.pool)
        .await?;
        touched.extend(tids);
        sqlx::query("DELETE FROM messages WHERE folder_id = ?")
            .bind(f.id)
            .execute(&ctx.pool)
            .await?;
        // Les UID ont changé : l'historique est à rapatrier depuis le début.
        sqlx::query("UPDATE folders SET backfill_next = NULL WHERE id = ?")
            .bind(f.id)
            .execute(&ctx.pool)
            .await?;
    }
    sqlx::query("UPDATE folders SET uid_validity = ?, uid_next = ?, highest_modseq = ?, total_count = ? WHERE id = ?")
        .bind(uid_validity).bind(uid_next).bind(mbox.highest_modseq.map(|m| m as i64)).bind(mbox.exists as i64).bind(f.id)
        .execute(&ctx.pool).await?;

    reconcile(session, ctx, f.id, touched, full).await?;

    // Nouveaux messages.
    let last_uid: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(uid), 0) FROM messages WHERE folder_id = ?")
            .bind(f.id)
            .fetch_one(&ctx.pool)
            .await?;
    let start = if last_uid > 0 {
        last_uid + 1
    } else {
        uid_next.map(|n| (n - INITIAL_WINDOW).max(1)).unwrap_or(1)
    };
    let mut count = 0;
    if uid_next.map_or(mbox.exists > 0, |n| start < n) {
        let mut stream = session
            .uid_fetch(
                format!("{start}:*"),
                "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[])",
            )
            .await?;
        while let Some(item) = stream.try_next().await? {
            if !item.uid.is_some_and(|u| i64::from(u) >= start) {
                continue; // « n:* » renvoie toujours le dernier message, même plus ancien que n
            }
            let Some(stored) = ingest_item(ctx, f, &item, false).await? else {
                continue;
            };
            // Notifier seulement l'arrivée d'un nouveau message dans la boîte de réception : pas la
            // première synchro (last_uid = 0), qui rapatrie l'historique.
            if last_uid > 0 && f.role.as_deref() == Some("inbox") && !stored.seen {
                fresh.push(FreshMail {
                    from: stored.from,
                    subject: stored.subject,
                });
            }
            let tid = stored.thread_id;
            touched.insert(tid);
            count += 1;
        }
    }

    ingest::refresh_folder_unread(&ctx.pool, f.id).await?;
    Ok(count)
}

/// Ce que la synchro retient d'un message qu'elle vient de ranger.
struct Stored {
    thread_id: i64,
    seen: bool,
    from: String,
    subject: String,
}

/// Range un message récupéré en entier (`UID FETCH … BODY.PEEK[]`) : le brut d'abord (source de
/// vérité), puis l'index. `lazy_attachments` : voir `ingest::StoreOpts`. `None` si l'élément n'a ni
/// UID ni corps, ou si le message est illisible.
async fn ingest_item(
    ctx: &Ctx,
    f: &FolderRec,
    item: &Fetch,
    lazy_attachments: bool,
) -> Result<Option<Stored>> {
    let (Some(uid), Some(raw)) = (item.uid.map(i64::from), item.body()) else {
        return Ok(None);
    };
    let Some(parsed) = ingest::parse(raw, item.internal_date().map(|d| d.timestamp())) else {
        tracing::warn!(folder = %f.path, uid, "message illisible ignoré");
        return Ok(None);
    };
    let flags = ingest::Flags {
        seen: item.flags().any(|fl| matches!(fl, Flag::Seen)),
        flagged: item.flags().any(|fl| matches!(fl, Flag::Flagged)),
    };
    let from = parsed
        .from_name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| parsed.from_addr.clone());
    let subject = parsed.subject.clone();
    let raw_hash = rawstore::put(&ctx.raw_dir, raw).await?;
    let thread_id = ingest::store_with(
        &ctx.pool,
        &ctx.attachments_dir,
        ingest::MessageLoc {
            account_id: &ctx.account_id,
            folder_id: f.id,
            uid,
            size: item.size.map(i64::from),
        },
        flags,
        parsed,
        ingest::StoreOpts {
            raw_hash: Some(&raw_hash),
            lazy_attachments,
        },
    )
    .await?;
    Ok(Some(Stored {
        thread_id,
        seen: flags.seen,
        from,
        subject,
    }))
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct BackfillEvent {
    account_id: String,
    /// Messages du serveur pas encore rapatriés (0 : historique complet).
    remaining: i64,
    total: i64,
}

/// Rapatrie l'historique : d'abord les messages sans brut (d'avant le magasin), puis les lots de
/// plus en plus anciens de chaque dossier. Au moins un lot par appel, au plus `BACKFILL_BUDGET`.
/// Retourne `true` s'il reste du travail.
async fn backfill(session: &mut ImapSession, ctx: &Ctx, folders: &[FolderRec]) -> Result<bool> {
    let started = Instant::now();
    let mut touched = HashSet::new();
    let mut ran = false;
    let more = loop {
        let more = rehydrate(session, ctx, folders).await?
            || backfill_batch(session, ctx, folders, &mut touched).await?;
        ran |= more;
        if !more || started.elapsed() >= BACKFILL_BUDGET {
            break more;
        }
        // Une demande manuelle ou un réveil : on la rend à la boucle principale, qui la traitera.
        if ctx.notify.notified().now_or_never().is_some() {
            ctx.notify.notify_one();
            break true;
        }
    };
    if !ran {
        return Ok(false); // rien à rapatrier (cas courant) : ni requête ni événement
    }
    if !touched.is_empty() {
        ingest::refresh_threads(&ctx.pool, &touched).await?;
    }
    let (remaining, total): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(MAX(f.total_count - (SELECT COUNT(*) FROM messages m WHERE m.folder_id = f.id), 0)), 0), \
                COALESCE(SUM(f.total_count), 0) \
         FROM folders f WHERE f.account_id = ? AND COALESCE(f.role, '') <> 'all'",
    )
    .bind(&ctx.account_id)
    .fetch_one(&ctx.pool)
    .await?;
    let _ = ctx.app.emit(
        "mail:backfill",
        BackfillEvent {
            account_id: ctx.account_id.clone(),
            remaining: if more { remaining.max(1) } else { 0 },
            total,
        },
    );
    Ok(more)
}

/// Récupère le message brut des messages qui n'en ont pas (synchronisés avant le magasin brut) et
/// relit leurs en-têtes de désabonnement. Un lot par appel ; `true` s'il a travaillé.
async fn rehydrate(session: &mut ImapSession, ctx: &Ctx, folders: &[FolderRec]) -> Result<bool> {
    let Some(folder_id) = sqlx::query_scalar::<_, i64>(
        "SELECT folder_id FROM messages WHERE account_id = ? AND raw_hash IS NULL LIMIT 1",
    )
    .bind(&ctx.account_id)
    .fetch_optional(&ctx.pool)
    .await?
    else {
        return Ok(false);
    };
    let uids: Vec<i64> = sqlx::query_scalar(
        "SELECT uid FROM messages WHERE folder_id = ? AND raw_hash IS NULL ORDER BY uid DESC LIMIT ?",
    )
    .bind(folder_id)
    .bind(BACKFILL_BATCH)
    .fetch_all(&ctx.pool)
    .await?;
    let Some(folder) = folders.iter().find(|f| f.id == folder_id) else {
        // Dossier non synchronisé : rien à récupérer, on ne réessaie pas.
        sqlx::query("UPDATE messages SET raw_hash = '' WHERE folder_id = ? AND raw_hash IS NULL")
            .bind(folder_id)
            .execute(&ctx.pool)
            .await?;
        return Ok(true);
    };
    session.select(&folder.path).await?;
    let set = uids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut stream = session.uid_fetch(set, "(UID BODY.PEEK[])").await?;
    while let Some(item) = stream.try_next().await? {
        let (Some(uid), Some(raw)) = (item.uid, item.body()) else {
            continue;
        };
        let hash = rawstore::put(&ctx.raw_dir, raw).await?;
        let unsub = ingest::parse(raw, None).and_then(|m| m.unsubscribe);
        sqlx::query(
            "UPDATE messages SET raw_hash = ?, unsub_url = ?, unsub_mailto = ?, unsub_one_click = ? \
             WHERE folder_id = ? AND uid = ?",
        )
        .bind(hash)
        .bind(unsub.as_ref().and_then(|u| u.url.as_deref()))
        .bind(unsub.as_ref().and_then(|u| u.mailto.as_deref()))
        .bind(unsub.as_ref().is_some_and(|u| u.one_click))
        .bind(folder_id)
        .bind(i64::from(uid))
        .execute(&ctx.pool)
        .await?;
    }
    drop(stream);
    // Absents du serveur (supprimés depuis) : on ne les redemande pas.
    sqlx::query("UPDATE messages SET raw_hash = '' WHERE folder_id = ? AND raw_hash IS NULL AND uid IN (SELECT value FROM json_each(?))")
        .bind(folder_id)
        .bind(serde_json::to_string(&uids)?)
        .execute(&ctx.pool)
        .await?;
    Ok(true)
}

/// Un lot de l'historique du premier dossier incomplet. `true` s'il y a eu (ou s'il reste) du travail.
async fn backfill_batch(
    session: &mut ImapSession,
    ctx: &Ctx,
    folders: &[FolderRec],
    touched: &mut HashSet<i64>,
) -> Result<bool> {
    for f in folders {
        let stored: Option<i64> =
            sqlx::query_scalar("SELECT backfill_next FROM folders WHERE id = ?")
                .bind(f.id)
                .fetch_one(&ctx.pool)
                .await?;
        let cursor = match stored {
            Some(c) => c,
            None => {
                let min: Option<i64> =
                    sqlx::query_scalar("SELECT MIN(uid) FROM messages WHERE folder_id = ?")
                        .bind(f.id)
                        .fetch_one(&ctx.pool)
                        .await?;
                match min {
                    Some(m) => m,
                    None => continue, // dossier vide en local : rien avant
                }
            }
        };
        if cursor <= 1 {
            continue;
        }
        let lo = (cursor - BACKFILL_BATCH).max(1);
        session.select(&f.path).await?;
        let mut stream = session
            .uid_fetch(
                format!("{lo}:{}", cursor - 1),
                "(UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[])",
            )
            .await?;
        while let Some(item) = stream.try_next().await? {
            let Some(stored) = ingest_item(ctx, f, &item, true).await? else {
                continue;
            };
            let tid = stored.thread_id;
            touched.insert(tid);
        }
        drop(stream);
        sqlx::query("UPDATE folders SET backfill_next = ? WHERE id = ?")
            .bind(lo)
            .bind(f.id)
            .execute(&ctx.pool)
            .await?;
        ingest::refresh_folder_unread(&ctx.pool, f.id).await?;
        return Ok(true);
    }
    Ok(false)
}

/// Aligne flags (lu/suivi) et suppressions sur la fenêtre de messages déjà en cache.
async fn reconcile(
    session: &mut ImapSession,
    ctx: &Ctx,
    folder_id: i64,
    touched: &mut HashSet<i64>,
    full: bool,
) -> Result<()> {
    let min_uid: Option<i64> =
        sqlx::query_scalar("SELECT MIN(uid) FROM messages WHERE folder_id = ?")
            .bind(folder_id)
            .fetch_one(&ctx.pool)
            .await?;
    let Some(min_uid) = min_uid else {
        return Ok(());
    };
    // Hors passe complète, on ne réaligne que les messages récents : avec l'historique rapatrié,
    // relire les drapeaux de dizaines de milliers de messages à chaque réveil serait du gâchis.
    let min_uid = if full {
        min_uid
    } else {
        sqlx::query_scalar::<_, i64>(
            "SELECT uid FROM messages WHERE folder_id = ? ORDER BY uid DESC LIMIT 1 OFFSET ?",
        )
        .bind(folder_id)
        .bind(RECONCILE_RECENT)
        .fetch_optional(&ctx.pool)
        .await?
        .unwrap_or(min_uid)
    };

    let mut server: HashMap<i64, (bool, bool)> = HashMap::new();
    {
        let mut stream = session
            .uid_fetch(format!("{min_uid}:*"), "(UID FLAGS)")
            .await?;
        while let Some(item) = stream.try_next().await? {
            if let Some(uid) = item.uid {
                server.insert(
                    uid as i64,
                    (
                        item.flags().any(|f| matches!(f, Flag::Seen)),
                        item.flags().any(|f| matches!(f, Flag::Flagged)),
                    ),
                );
            }
        }
    }
    let local: Vec<(i64, i64, bool, bool, Option<i64>)> = sqlx::query_as(
        "SELECT id, uid, is_read, is_starred, thread_id FROM messages WHERE folder_id = ? AND uid >= ?",
    )
    .bind(folder_id)
    .bind(min_uid)
    .fetch_all(&ctx.pool)
    .await?;
    let mut tx = ctx.pool.begin().await?;
    for (id, uid, read, starred, thread) in local {
        match server.get(&uid) {
            None => {
                sqlx::query("DELETE FROM messages WHERE id = ?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                touched.extend(thread);
            }
            Some(&(r, s)) if r != read || s != starred => {
                sqlx::query("UPDATE messages SET is_read = ?, is_starred = ? WHERE id = ?")
                    .bind(r)
                    .bind(s)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                touched.extend(thread);
            }
            _ => {}
        }
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::decode_mutf7;

    #[test]
    fn mutf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("Envoy&AOk-s"), "Envoyés");
        assert_eq!(decode_mutf7("A &- B"), "A & B");
        assert_eq!(decode_mutf7("&2D3eAA-"), "\u{1F600}"); // paire de substitution UTF-16
    }
}
