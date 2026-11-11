//! Import des dossiers locaux d'un autre client (Thunderbird : mbox ou Maildir) dans « Sur cet ordinateur ».
//!
//! Les messages sont repris **tels quels** (message brut dans `rawstore`), l'index est reconstruit
//! à partir d'eux. L'import est idempotent : un message déjà présent dans le dossier (même empreinte)
//! n'est pas dupliqué, on peut donc relancer après une interruption.
//!
//! Le découpage des fichiers mbox est fait ici, en flux, plutôt que par une bibliothèque : les
//! lecteurs existants projettent le fichier en mémoire (`mmap`), ce qui fait planter le processus si
//! Thunderbird modifie ou tronque le fichier pendant la lecture, et chargent tout le fichier.
//! Le format est simple (RFC 4155, variante « libérale ») et couvert par des tests.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{NaiveDateTime, TimeZone, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::error::{AppError, Result};
use crate::mail::account::LOCAL_ID;
use crate::mail::{ingest, local, rawstore};

/// Taille maximale d'un message importé (au-delà : ignoré et compté en échec).
const MAX_MESSAGE: usize = 256 * 1024 * 1024;
const MAX_DEPTH: usize = 8;
const MAX_FOLDERS: usize = 5000;
/// Séparateur de hiérarchie dans le nom d'un dossier local (qui est plat).
const SEP: &str = " › ";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Mbox,
    Maildir,
}

/// Un dossier trouvé dans la source.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// Identifiant stable : chemin relatif joint par `/`.
    pub key: String,
    /// Nom du dossier local créé : hiérarchie jointe par « › ».
    pub name: String,
    pub kind: Kind,
    /// Octets (mbox) ou nombre de fichiers (Maildir).
    pub size: u64,
    /// Proposé par défaut (corbeille et boîte d'envoi temporaire exclues).
    pub selected: bool,
    #[serde(skip)]
    path: PathBuf,
}

/// Dossier à parcourir à partir de ce que l'utilisateur a choisi : un profil Thunderbird
/// (`Mail/Local Folders`), le dossier `Mail`, ou directement un dossier de boîtes.
fn base_dir(root: &Path) -> PathBuf {
    [
        root.join("Mail").join("Local Folders"),
        root.join("Local Folders"),
    ]
    .into_iter()
    .find(|p| p.is_dir())
    .unwrap_or_else(|| root.to_path_buf())
}

/// Fichiers d'index ou de données d'un client mail : jamais des boîtes. Les autres sont reconnus à leur
/// contenu (une archive `.txt` ou `.mbox` qui commence par « From » est un mbox).
fn ignorable(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.starts_with('.')
        || [".msf", ".dat", ".sqlite", ".db", ".lock"]
            .iter()
            .any(|e| lower.ends_with(e))
}

async fn is_mbox(path: &Path) -> bool {
    let Ok(mut f) = tokio::fs::File::open(path).await else {
        return false;
    };
    let mut head = [0u8; 5];
    f.read_exact(&mut head).await.is_ok() && &head == b"From "
}

fn is_maildir(path: &Path) -> bool {
    path.join("cur").is_dir() && path.join("new").is_dir()
}

async fn maildir_files(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["cur", "new"] {
        if let Ok(mut rd) = tokio::fs::read_dir(path.join(sub)).await {
            while let Ok(Some(e)) = rd.next_entry().await {
                let name = e.file_name().to_string_lossy().into_owned();
                if !name.starts_with('.') && e.file_type().await.is_ok_and(|t| t.is_file()) {
                    out.push(e.path());
                }
            }
        }
    }
    out.sort();
    out
}

/// Parcourt la source et liste les dossiers importables (sans lire les messages).
pub async fn discover(root: &Path) -> Result<Vec<Found>> {
    let base = base_dir(root);
    if !base.is_dir() {
        return Err(AppError::Invalid("ce n'est pas un dossier".into()));
    }
    let mut out = Vec::new();
    walk(&base, &mut Vec::new(), &mut out).await?;
    out.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(out)
}

fn walk<'a>(
    dir: &'a Path,
    parents: &'a mut Vec<String>,
    out: &'a mut Vec<Found>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        if parents.len() >= MAX_DEPTH || out.len() >= MAX_FOLDERS {
            return Ok(());
        }
        let mut entries = Vec::new();
        let mut rd = tokio::fs::read_dir(dir).await?;
        while let Some(e) = rd.next_entry().await? {
            // Pas de lien symbolique : la source ne doit pas faire sortir de son dossier.
            if let Ok(t) = e.file_type().await {
                if !t.is_symlink() {
                    entries.push((
                        e.file_name().to_string_lossy().into_owned(),
                        e.path(),
                        t.is_dir(),
                    ));
                }
            }
        }
        entries.sort();
        let names: HashSet<String> = entries.iter().map(|(n, _, _)| n.clone()).collect();
        for (name, path, is_dir) in &entries {
            if ignorable(name) {
                continue;
            }
            if let Some(base) = name.strip_suffix(".sbd").filter(|_| *is_dir) {
                // Sous-dossiers de `base` ; ceux d'un dossier qui a son propre fichier sont vus avec lui.
                if !names.contains(base) {
                    parents.push(base.to_string());
                    walk(path, parents, out).await?;
                    parents.pop();
                }
                continue;
            }
            let found = if *is_dir && is_maildir(path) {
                Some((Kind::Maildir, maildir_files(path).await.len() as u64))
            } else if !*is_dir && is_mbox(path).await {
                Some((Kind::Mbox, tokio::fs::metadata(path).await?.len()))
            } else {
                None
            };
            match found {
                Some((kind, size)) => {
                    let mut parts = parents.clone();
                    parts.push(name.clone());
                    out.push(Found {
                        key: parts.join("/"),
                        name: local::sanitize_name(&parts.join(SEP)),
                        kind,
                        size,
                        // Corbeille, indésirables et boîte d'envoi temporaire : pas proposés par défaut.
                        selected: !matches!(
                            name.to_lowercase().as_str(),
                            "trash" | "junk" | "unsent messages"
                        ),
                        path: path.clone(),
                    });
                    let sub = dir.join(format!("{name}.sbd"));
                    if names.contains(&format!("{name}.sbd")) && sub.is_dir() {
                        parents.push(name.clone());
                        walk(&sub, parents, out).await?;
                        parents.pop();
                    }
                }
                None if *is_dir && !is_maildir(path) => {
                    // Dossier ordinaire (source qui n'est pas un profil) : on y cherche des boîtes.
                    parents.push(name.clone());
                    walk(path, parents, out).await?;
                    parents.pop();
                }
                None => {}
            }
        }
        Ok(())
    })
}

/// Un message d'un fichier mbox.
#[derive(Debug, PartialEq, Eq)]
pub struct MboxItem {
    /// Ligne « From … » qui précède le message (sans fin de ligne).
    pub from_line: String,
    /// Le message, sans la ligne « From » ni la ligne vide qui le sépare du suivant.
    pub raw: Vec<u8>,
    /// Dépassait `MAX_MESSAGE` : contenu non conservé.
    pub too_big: bool,
}

/// Découpe un mbox en flux : un message commence par une ligne `From ` en début de fichier ou après
/// une ligne vide. Les lignes `>From ` du corps ne sont pas modifiées (le message reste tel quel).
pub struct MboxReader<R> {
    reader: BufReader<R>,
    pending_from: Option<String>,
    prev_blank: bool,
    started: bool,
}

impl<R: tokio::io::AsyncRead + Unpin> MboxReader<R> {
    pub fn new(r: R) -> Self {
        Self {
            reader: BufReader::new(r),
            pending_from: None,
            prev_blank: true,
            started: false,
        }
    }

    pub async fn next(&mut self) -> Result<Option<MboxItem>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut too_big = false;
        let mut line = Vec::new();
        let from_line = loop {
            if let Some(f) = self.pending_from.take() {
                break f;
            }
            line.clear();
            if self.reader.read_until(b'\n', &mut line).await? == 0 {
                return Ok(None); // fin sans message
            }
            if line.starts_with(b"From ") && self.prev_blank {
                self.started = true;
                break trim_eol(&line);
            }
            // Octets avant le premier « From » : ignorés.
            self.prev_blank = is_blank(&line);
        };
        self.prev_blank = false;
        loop {
            line.clear();
            if self.reader.read_until(b'\n', &mut line).await? == 0 {
                break;
            }
            let blank = is_blank(&line);
            if line.starts_with(b"From ") && self.prev_blank {
                self.pending_from = Some(trim_eol(&line));
                self.prev_blank = false;
                break;
            }
            if !too_big {
                if buf.len() + line.len() > MAX_MESSAGE {
                    too_big = true;
                    buf = Vec::new();
                } else {
                    buf.extend_from_slice(&line);
                }
            }
            self.prev_blank = blank;
        }
        // La dernière ligne vide sépare ce message du suivant : elle n'en fait pas partie.
        if buf.ends_with(b"\r\n\r\n") {
            buf.truncate(buf.len() - 2);
        } else if buf.ends_with(b"\n\n") {
            buf.truncate(buf.len() - 1);
        }
        Ok(Some(MboxItem {
            from_line,
            raw: buf,
            too_big,
        }))
    }
}

/// Ligne vide (fin de ligne seule) : test sur les octets, sans allouer de chaîne pour chaque ligne du fichier.
fn is_blank(line: &[u8]) -> bool {
    line.iter().all(|b| matches!(b, b'\r' | b'\n'))
}

fn trim_eol(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

/// Drapeaux Thunderbird (`X-Mozilla-Status`, hexadécimal) : lu = 0x1, suivi = 0x4, supprimé (« expunged ») = 0x8.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MozFlags {
    pub read: bool,
    pub starred: bool,
    pub deleted: bool,
}

pub fn mozilla_flags(raw: &[u8]) -> Option<MozFlags> {
    const KEY: &[u8] = b"x-mozilla-status:";
    for line in raw.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break; // fin des en-têtes
        }
        if line.len() > KEY.len() && line[..KEY.len()].eq_ignore_ascii_case(KEY) {
            let bits =
                u32::from_str_radix(std::str::from_utf8(&line[KEY.len()..]).ok()?.trim(), 16)
                    .ok()?;
            return Some(MozFlags {
                read: bits & 0x1 != 0,
                starred: bits & 0x4 != 0,
                deleted: bits & 0x8 != 0,
            });
        }
    }
    None
}

/// Date de la ligne « From … Wed Mar  2 10:00:00 2026 », utile si le message n'a pas d'en-tête Date.
pub fn from_line_timestamp(from_line: &str) -> Option<i64> {
    let t: Vec<&str> = from_line.split_whitespace().collect();
    let n = t.len();
    if n < 5 {
        return None;
    }
    let text = format!("{} {} {} {}", t[n - 4], t[n - 3], t[n - 2], t[n - 1]);
    let d = NaiveDateTime::parse_from_str(&text, "%b %d %H:%M:%S %Y").ok()?;
    Some(Utc.from_utc_datetime(&d).timestamp())
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub imported: usize,
    pub duplicates: usize,
    /// Messages supprimés dans la source (marqués « expunged »), non repris.
    pub deleted: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub folder: String,
    pub imported: usize,
    pub folders_done: usize,
    pub folders_total: usize,
}

/// Un message à ranger, tel que la source le fournit.
struct Incoming<'a> {
    raw: &'a [u8],
    /// Ligne « From » du mbox (sa date sert si le message n'en a pas).
    from_line: Option<&'a str>,
    /// Drapeaux de la source ; à défaut, ceux de `X-Mozilla-Status`.
    flags: Option<MozFlags>,
}

/// Destination d'un import : un dossier local, le rapport et les conversations à rafraîchir.
struct Sink<'a> {
    pool: &'a SqlitePool,
    att_dir: &'a Path,
    raw_dir: &'a Path,
    folder: i64,
    report: &'a mut Report,
    touched: HashSet<i64>,
}

impl Sink<'_> {
    /// Range un message : brut, puis index. Ne duplique rien (même empreinte dans le dossier).
    async fn add(&mut self, m: Incoming<'_>) -> Result<()> {
        let flags = m.flags.or_else(|| mozilla_flags(m.raw)).unwrap_or_default();
        if flags.deleted {
            self.report.deleted += 1;
            return Ok(());
        }
        let fallback = m.from_line.and_then(from_line_timestamp);
        // Pièces jointes extraites à la demande : on ne les copie pas en mémoire pour les jeter.
        let Some(parsed) = ingest::parse_with(m.raw, fallback, false) else {
            self.report.failed += 1;
            return Ok(());
        };
        // En-tête interne de certains fichiers (Netscape/SeaMonkey) : pas un vrai message.
        if parsed.subject.starts_with("DON'T DELETE THIS MESSAGE") {
            return Ok(());
        }
        let hash = rawstore::put(self.raw_dir, m.raw).await?;
        if ingest::has_raw(self.pool, self.folder, &hash).await? {
            self.report.duplicates += 1;
            return Ok(());
        }
        let tid = ingest::store_with(
            self.pool,
            self.att_dir,
            ingest::MessageLoc {
                account_id: LOCAL_ID,
                folder_id: self.folder,
                // Calculé au dernier moment : un import long ne bloque pas les autres actions.
                uid: ingest::next_uid(self.pool, self.folder).await?,
                size: Some(m.raw.len() as i64),
            },
            ingest::Flags {
                seen: flags.read,
                flagged: flags.starred,
            },
            parsed,
            ingest::StoreOpts {
                raw_hash: Some(&hash),
                lazy_attachments: true,
            },
        )
        .await?;
        self.touched.insert(tid);
        self.report.imported += 1;
        Ok(())
    }

    /// Rafraîchit les conversations et signale l'avancement par lots (pas à chaque message).
    async fn flush(&mut self, tick: &(dyn Fn(usize) + Sync)) -> Result<()> {
        if self.touched.len() >= 200 {
            ingest::refresh_threads(self.pool, &self.touched).await?;
            self.touched.clear();
            tick(self.report.imported);
        }
        Ok(())
    }

    async fn finish(self) -> Result<()> {
        ingest::refresh_threads(self.pool, &self.touched).await?;
        ingest::refresh_folder_unread(self.pool, self.folder).await
    }
}

/// Drapeaux d'un fichier Maildir : ceux de Thunderbird s'il en a écrit, sinon le suffixe du nom
/// (`…:2,SF`, ou `…;2,SF` là où `:` est interdit, comme sous Windows).
fn maildir_flags(file: &Path, raw: &[u8]) -> MozFlags {
    mozilla_flags(raw).unwrap_or_else(|| {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let suffix = name
            .rsplit_once(":2,")
            .or_else(|| name.rsplit_once(";2,"))
            .map_or("", |(_, s)| s);
        MozFlags {
            read: suffix.contains('S'),
            starred: suffix.contains('F'),
            deleted: suffix.contains('T'),
        }
    })
}

/// Importe les dossiers choisis (`keys`, issus de `discover`) dans des dossiers locaux de même nom.
pub async fn import(
    pool: &SqlitePool,
    att_dir: &Path,
    raw_dir: &Path,
    root: &Path,
    keys: &HashSet<String>,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<Report> {
    let found: Vec<Found> = discover(root)
        .await?
        .into_iter()
        .filter(|f| keys.contains(&f.key))
        .collect();
    let total = found.len();
    let mut report = Report::default();
    for (i, f) in found.iter().enumerate() {
        let folder = local::get_or_create_folder(pool, &f.name).await?;
        let tick = |imported: usize| {
            progress(Progress {
                folder: f.name.clone(),
                imported,
                folders_done: i,
                folders_total: total,
            })
        };
        tick(report.imported);
        let mut sink = Sink {
            pool,
            att_dir,
            raw_dir,
            folder,
            report: &mut report,
            touched: HashSet::new(),
        };
        match f.kind {
            Kind::Mbox => {
                let mut r = MboxReader::new(tokio::fs::File::open(&f.path).await?);
                while let Some(item) = r.next().await? {
                    if item.too_big {
                        sink.report.failed += 1;
                        continue;
                    }
                    sink.add(Incoming {
                        raw: &item.raw,
                        from_line: Some(&item.from_line),
                        flags: None,
                    })
                    .await?;
                    sink.flush(&tick).await?;
                }
            }
            Kind::Maildir => {
                for file in maildir_files(&f.path).await {
                    if tokio::fs::metadata(&file).await?.len() as usize > MAX_MESSAGE {
                        sink.report.failed += 1;
                        continue;
                    }
                    let raw = tokio::fs::read(&file).await?;
                    let flags = Some(maildir_flags(&file, &raw));
                    sink.add(Incoming {
                        raw: &raw,
                        from_line: None,
                        flags,
                    })
                    .await?;
                    sink.flush(&tick).await?;
                }
            }
        }
        sink.finish().await?;
    }
    progress(Progress {
        folder: String::new(),
        imported: report.imported,
        folders_done: total,
        folders_total: total,
    });
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(subject: &str, status: &str, body: &str) -> String {
        format!("X-Mozilla-Status: {status}\nFrom: a@x.fr\nTo: me@x.fr\nSubject: {subject}\nDate: Mon, 2 Mar 2026 10:00:00 +0000\nMessage-ID: <{subject}@x>\n\n{body}\n")
    }
    fn mbox(items: &[String]) -> String {
        items
            .iter()
            .map(|m| format!("From - Mon Mar  2 10:00:00 2026\n{m}\n"))
            .collect()
    }
    async fn split(data: &[u8]) -> Vec<MboxItem> {
        let mut r = MboxReader::new(data);
        let mut out = Vec::new();
        while let Some(i) = r.next().await.unwrap() {
            out.push(i);
        }
        out
    }

    #[tokio::test]
    async fn decoupage_mbox() {
        let data = mbox(&[
            msg("un", "0001", "bonjour"),
            msg("deux", "0000", "salut\n\n>From ici\nfin"),
        ]);
        let items = split(data.as_bytes()).await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].from_line, "From - Mon Mar  2 10:00:00 2026");
        assert!(items[0].raw.starts_with(b"X-Mozilla-Status: 0001\n"));
        assert!(
            items[0].raw.ends_with(b"bonjour\n"),
            "la ligne vide séparatrice n'en fait pas partie"
        );
        // Une ligne « >From » du corps ne coupe pas le message et reste telle quelle.
        assert!(String::from_utf8_lossy(&items[1].raw).contains("\n>From ici\nfin\n"));
    }

    #[tokio::test]
    async fn mbox_cas_limites() {
        assert!(split(b"").await.is_empty());
        assert!(split(b"pas un mbox\n").await.is_empty());
        // Octets avant le premier « From », fin sans saut de ligne, fins de ligne CRLF.
        let data = b"preambule\n\nFrom a@x.fr Mon Mar  2 10:00:00 2026\r\nSubject: crlf\r\n\r\ncorps\r\n\r\nFrom b@x.fr Tue Mar  3 11:00:00 2026\r\nSubject: fin\r\n\r\ndernier";
        let items = split(data).await;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].raw, b"Subject: crlf\r\n\r\ncorps\r\n");
        assert_eq!(items[1].raw, b"Subject: fin\r\n\r\ndernier");
        // « From » collé à du texte (sans ligne vide avant) n'est pas un séparateur.
        let items =
            split(b"From x Mon Mar  2 10:00:00 2026\nSubject: s\n\nligne\nFrom la mer\nsuite\n")
                .await;
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn drapeaux_thunderbird_et_date_de_la_ligne_from() {
        assert_eq!(
            mozilla_flags(b"X-Mozilla-Status: 0005\nSubject: x\n\nbody"),
            Some(MozFlags {
                read: true,
                starred: true,
                deleted: false
            })
        );
        assert_eq!(
            mozilla_flags(b"X-Mozilla-Status: 0009\n\n").map(|f| f.deleted),
            Some(true)
        );
        assert_eq!(
            mozilla_flags(b"Subject: x\n\nX-Mozilla-Status: 0001\n"),
            None,
            "seulement dans les en-têtes"
        );
        assert_eq!(
            from_line_timestamp("From - Mon Mar  2 10:00:00 2026"),
            Some(1772445600)
        );
        assert_eq!(
            from_line_timestamp("From a@x.fr Mon Mar 02 10:00:00 2026"),
            Some(1772445600)
        );
        assert_eq!(from_line_timestamp("From -"), None);
    }

    /// Profil Thunderbird factice : mbox, sous-dossier `.sbd`, corbeille, Maildir, fichiers d'index.
    fn profile() -> PathBuf {
        let root = std::env::temp_dir().join(format!("envelope-tb-{}", uuid::Uuid::new_v4()));
        let lf = root.join("Mail").join("Local Folders");
        std::fs::create_dir_all(lf.join("Archives.sbd")).unwrap();
        std::fs::write(
            lf.join("Archives"),
            mbox(&[
                msg("a1", "0001", "un"),
                msg("a2", "0005", "deux"),
                msg("a3", "0009", "supprimé"),
            ]),
        )
        .unwrap();
        std::fs::write(lf.join("Archives.msf"), "index").unwrap();
        std::fs::write(
            lf.join("Archives.sbd").join("2023"),
            mbox(&[msg("y1", "0000", "sous-dossier")]),
        )
        .unwrap();
        std::fs::write(lf.join("Trash"), mbox(&[msg("t1", "0001", "poubelle")])).unwrap();
        std::fs::write(lf.join("notes.txt"), "pas un dossier").unwrap();
        std::fs::write(
            lf.join("liste.txt"),
            mbox(&[msg("l1", "0000", "archive de liste")]),
        )
        .unwrap();
        let md = lf.join("Projets");
        std::fs::create_dir_all(md.join("cur")).unwrap();
        std::fs::create_dir_all(md.join("new")).unwrap();
        std::fs::create_dir_all(md.join("tmp")).unwrap();
        std::fs::write(
            md.join("cur").join("1;2,SF"),
            msg("m1", "0000", "maildir").replace("X-Mozilla-Status: 0000\n", ""),
        )
        .unwrap();
        std::fs::write(
            md.join("new").join("2"),
            msg("m2", "0000", "nouveau").replace("X-Mozilla-Status: 0000\n", ""),
        )
        .unwrap();
        root
    }

    #[tokio::test]
    async fn decouverte_du_profil() {
        let root = profile();
        let found = discover(&root).await.unwrap();
        let view: Vec<(&str, &str, bool)> = found
            .iter()
            .map(|f| (f.key.as_str(), f.name.as_str(), f.selected))
            .collect();
        assert_eq!(
            view,
            [
                ("Archives", "Archives", true),
                ("Archives/2023", "Archives › 2023", true),
                ("Projets", "Projets", true),
                ("Trash", "Trash", false),
                ("liste.txt", "liste.txt", true)
            ]
        );
        // Le dossier « Mail » ou « Local Folders » directement donne le même résultat.
        assert_eq!(discover(&root.join("Mail")).await.unwrap().len(), 5);
        assert!(discover(&root.join("absent")).await.is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn import_idempotent_avec_drapeaux_et_hierarchie() {
        let root = profile();
        let pool = crate::db::init_memory().await.unwrap();
        let base = std::env::temp_dir().join(format!("envelope-tb-data-{}", uuid::Uuid::new_v4()));
        let keys: HashSet<String> = ["Archives", "Archives/2023", "Projets"]
            .map(String::from)
            .into();
        let noop = |_: Progress| {};

        let r = import(
            &pool,
            &base.join("att"),
            &base.join("raw"),
            &root,
            &keys,
            &noop,
        )
        .await
        .unwrap();
        assert_eq!(
            (r.imported, r.duplicates, r.deleted, r.failed),
            (5, 0, 1, 0),
            "le message « expunged » n'est pas ressuscité"
        );
        let counts: Vec<(String, i64)> = sqlx::query_as(
            "SELECT f.name, COUNT(m.id) FROM folders f LEFT JOIN messages m ON m.folder_id = f.id WHERE f.account_id = 'local' AND f.name <> 'Archives' OR f.name = 'Archives' GROUP BY f.id ORDER BY f.name",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(
            counts,
            [
                ("Archives".into(), 2),
                ("Archives › 2023".into(), 1),
                ("Projets".into(), 2)
            ]
        );
        // Drapeaux : Thunderbird (lu, lu + suivi) et Maildir (S = lu, F = suivi).
        let flags: Vec<(String, bool, bool)> =
            sqlx::query_as("SELECT subject, is_read, is_starred FROM messages ORDER BY subject")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            flags,
            [
                ("a1".into(), true, false),
                ("a2".into(), true, true),
                ("m1".into(), true, true),
                ("m2".into(), false, false),
                ("y1".into(), false, false)
            ]
        );
        let acct: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE account_id = 'local'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(acct, 5);

        // Relancer ne duplique rien.
        let again = import(
            &pool,
            &base.join("att"),
            &base.join("raw"),
            &root,
            &keys,
            &noop,
        )
        .await
        .unwrap();
        assert_eq!((again.imported, again.duplicates), (0, 5));
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(base);
    }
}
