//! Recherche plein texte (FTS5) et requêtes de lecture pour l'UI et le serveur MCP.

use serde::Serialize;
use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::Result;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummary {
    pub id: i64,
    pub account_id: String,
    pub folder_id: i64,
    pub thread_id: Option<i64>,
    pub subject: String,
    pub from_name: Option<String>,
    pub from_addr: String,
    pub date: String,
    pub snippet: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
}

const SUMMARY_COLS: &str =
    "m.id, m.account_id, m.folder_id, m.thread_id, m.subject, m.from_name, m.from_addr, \
    m.date, m.snippet, m.is_read, m.is_starred, m.has_attachments";

/// Transforme une saisie utilisateur en requête FTS5 sûre : chaque mot devient un préfixe entre
/// guillemets (`"mot"*`), combinés en ET. Neutralise les opérateurs FTS5 (`OR`, `NEAR`, `-`, `:`…).
pub fn build_fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|t| t.replace('"', ""))
        .filter(|t| t.chars().any(char::is_alphanumeric))
        .map(|t| format!("\"{t}\"*"))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// Motif `%…%` pour `LIKE … ESCAPE '\'`, avec `\`, `%` et `_` saisis par l'utilisateur neutralisés.
pub fn like_contains(s: &str) -> String {
    format!(
        "%{}%",
        s.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

/// Filtres tirés d'opérateurs de recherche (`de:`, `a:`, `objet:`, `avant:`, `apres:`, `pj`, `non-lu`, `suivi`).
/// Tous portent sur un même message : une conversation correspond si l'un de ses messages satisfait
/// l'ensemble des critères.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SearchSpec {
    /// Mots libres, destinés à la recherche plein texte.
    pub text: String,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    /// Bornes `YYYY[-MM[-DD]]` : `before` exclusive, `after` inclusive.
    pub before: Option<String>,
    pub after: Option<String>,
    /// `dans:` : dossiers visés (rôle ou nom), `dans:tout` pour inclure aussi corbeille et spam.
    pub folders: Vec<String>,
    pub anywhere: bool,
    pub attachment: bool,
    pub unread: bool,
    pub starred: bool,
}

/// Minuscules sans accents courants (comparaison de noms de dossiers saisis à la main).
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'à' | 'â' | 'ä' => 'a',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

/// Rôle de dossier désigné par un mot courant (« archives », « envoyés », « spam »…).
fn folder_role(word: &str) -> Option<&'static str> {
    Some(match fold(word).as_str() {
        "inbox" | "reception" | "boite" => "inbox",
        "archive" | "archives" => "archive",
        "sent" | "envoye" | "envoyes" => "sent",
        "drafts" | "brouillon" | "brouillons" => "drafts",
        "trash" | "corbeille" => "trash",
        "junk" | "spam" | "indesirable" | "indesirables" => "junk",
        _ => return None,
    })
}

/// `YYYY`, `YYYY-MM` ou `YYYY-MM-DD` valide (comparable à une date ISO, par ordre lexicographique).
fn valid_date(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    let num = |p: &str, len: usize| p.len() == len && p.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [y] => num(y, 4),
        [y, m] => num(y, 4) && num(m, 2) && (1..=12).contains(&m.parse::<u8>().unwrap_or(0)),
        [y, m, d] => {
            num(y, 4)
                && num(m, 2)
                && num(d, 2)
                && (1..=12).contains(&m.parse::<u8>().unwrap_or(0))
                && (1..=31).contains(&d.parse::<u8>().unwrap_or(0))
        }
        _ => false,
    }
}

/// Découpe sur les espaces hors guillemets ; les guillemets sont retirés.
fn tokens(input: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted) = (Vec::new(), String::new(), false);
    for c in input.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Analyse une saisie : opérateurs en français ou en anglais (`de:`/`from:`, `a:`/`à:`/`to:`,
/// `objet:`/`subject:`, `avant:`/`before:`, `apres:`/`après:`/`after:`, `pj`/`has:attachment`,
/// `non-lu`/`is:unread`, `suivi`/`is:starred`). Ce qui n'est pas un opérateur valide reste du texte libre.
pub fn parse_search(input: &str) -> SearchSpec {
    let mut spec = SearchSpec::default();
    let mut words = Vec::new();
    for tok in tokens(input) {
        if !apply_operator(&mut spec, &tok) {
            words.push(tok);
        }
    }
    spec.text = words.join(" ");
    spec
}

/// Applique `tok` à `spec` s'il s'agit d'un opérateur valide ; sinon `false` (c'est du texte libre).
fn apply_operator(spec: &mut SearchSpec, tok: &str) -> bool {
    let Some((key, value)) = tok.split_once(':') else {
        match tok.to_lowercase().as_str() {
            "pj" => spec.attachment = true,
            "non-lu" | "nonlu" => spec.unread = true,
            "suivi" | "favori" => spec.starred = true,
            _ => return false,
        }
        return true;
    };
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let lower = value.to_lowercase();
    match key.to_lowercase().as_str() {
        "de" | "from" => spec.from.push(value.into()),
        "a" | "à" | "to" => spec.to.push(value.into()),
        "objet" | "subject" => spec.subject.push(value.into()),
        "dans" | "in"
            if matches!(
                fold(value).as_str(),
                "tout" | "partout" | "anywhere" | "all"
            ) =>
        {
            spec.anywhere = true
        }
        "dans" | "in" => spec.folders.push(value.into()),
        "avant" | "before" if valid_date(value) => spec.before = Some(value.into()),
        "apres" | "après" | "after" if valid_date(value) => spec.after = Some(value.into()),
        "has" if matches!(lower.as_str(), "attachment" | "pj") => spec.attachment = true,
        "is" | "est" if matches!(lower.as_str(), "unread" | "nonlu" | "non-lu") => {
            spec.unread = true
        }
        "is" | "est" if matches!(lower.as_str(), "starred" | "suivi" | "favori") => {
            spec.starred = true
        }
        _ => return false,
    }
    true
}

/// Ajoute à `qb` (déjà positionné dans un `WHERE`) les conditions de `spec` sur la table `messages`
/// aliasée `m` : texte libre via FTS5, puis filtres. `false` si la saisie de texte n'est pas interrogeable.
pub fn push_message_filters(qb: &mut QueryBuilder<'_, Sqlite>, spec: &SearchSpec) -> bool {
    if !spec.text.trim().is_empty() {
        let Some(fts) = build_fts_query(&spec.text) else {
            return false;
        };
        qb.push(" AND m.id IN (SELECT rowid FROM messages_fts WHERE messages_fts MATCH ")
            .push_bind(fts)
            .push(")");
    }
    // Dossiers : tous par défaut, sauf corbeille et spam (comme Gmail) ; `dans:` cible un dossier,
    // `dans:tout` inclut tout.
    if spec.folders.is_empty() {
        if !spec.anywhere {
            qb.push(
                " AND m.folder_id NOT IN (SELECT id FROM folders WHERE role IN ('trash', 'junk'))",
            );
        }
    } else {
        qb.push(" AND m.folder_id IN (SELECT id FROM folders WHERE 0 = 1");
        for v in &spec.folders {
            match folder_role(v) {
                Some(role) => {
                    qb.push(" OR role = ").push_bind(role);
                }
                None => {
                    qb.push(" OR name LIKE ")
                        .push_bind(like_contains(v))
                        .push(" ESCAPE '\\'");
                }
            }
        }
        qb.push(")");
    }
    for v in &spec.from {
        let like = like_contains(v);
        qb.push(" AND (m.from_addr LIKE ")
            .push_bind(like.clone())
            .push(" ESCAPE '\\' OR m.from_name LIKE ")
            .push_bind(like)
            .push(" ESCAPE '\\')");
    }
    for v in &spec.to {
        // `to_addrs` / `cc_addrs` : JSON (adresse et nom).
        let like = like_contains(v);
        qb.push(" AND (m.to_addrs LIKE ")
            .push_bind(like.clone())
            .push(" ESCAPE '\\' OR m.cc_addrs LIKE ")
            .push_bind(like)
            .push(" ESCAPE '\\')");
    }
    for v in &spec.subject {
        qb.push(" AND m.subject LIKE ")
            .push_bind(like_contains(v))
            .push(" ESCAPE '\\'");
    }
    if let Some(d) = &spec.before {
        qb.push(" AND m.date < ").push_bind(d.clone());
    }
    if let Some(d) = &spec.after {
        qb.push(" AND m.date >= ").push_bind(d.clone());
    }
    if spec.attachment {
        qb.push(" AND m.has_attachments = 1");
    }
    if spec.unread {
        qb.push(" AND m.is_read = 0");
    }
    if spec.starred {
        qb.push(" AND m.is_starred = 1");
    }
    true
}

pub async fn search(
    pool: &SqlitePool,
    query: &str,
    account_id: Option<&str>,
    limit: i64,
) -> Result<Vec<MessageSummary>> {
    let Some(fts) = build_fts_query(query) else {
        return Ok(vec![]);
    };
    sqlx::query_as(&format!(
        "SELECT {SUMMARY_COLS} FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid \
         WHERE messages_fts MATCH ?1 AND (?2 IS NULL OR m.account_id = ?2) \
         ORDER BY rank LIMIT ?3"
    ))
    .bind(fts)
    .bind(account_id)
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await
}

pub async fn list_messages(
    pool: &SqlitePool,
    folder_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<MessageSummary>> {
    sqlx::query_as(&format!(
        "SELECT {SUMMARY_COLS} FROM messages m WHERE m.folder_id = ? ORDER BY m.date DESC LIMIT ? OFFSET ?"
    ))
    .bind(folder_id)
    .bind(limit.clamp(1, 500))
    .bind(offset.max(0))
    .fetch_all(pool)
    .await
}

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FolderRow {
    pub id: i64,
    pub account_id: String,
    pub path: String,
    pub name: String,
    pub role: Option<String>,
    pub unread_count: i64,
    pub total_count: i64,
}

pub async fn list_folders(pool: &SqlitePool, account_id: Option<&str>) -> Result<Vec<FolderRow>> {
    sqlx::query_as(
        "SELECT id, account_id, path, name, role, unread_count, total_count FROM folders \
         WHERE (?1 IS NULL OR account_id = ?1) \
         ORDER BY account_id, CASE role WHEN 'inbox' THEN 0 WHEN 'drafts' THEN 1 WHEN 'sent' THEN 2 \
         WHEN 'archive' THEN 3 WHEN 'junk' THEN 4 WHEN 'trash' THEN 5 ELSE 6 END, name",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operateurs_francais_et_anglais() {
        let s = parse_search(
            r#"budget de:"Hugo Martin" à:léa objet:devis avant:2026-03 apres:2025-01-15 pj non-lu suivi"#,
        );
        assert_eq!(s.text, "budget");
        assert_eq!(s.from, ["Hugo Martin"]);
        assert_eq!(s.to, ["léa"]);
        assert_eq!(s.subject, ["devis"]);
        assert_eq!(s.before.as_deref(), Some("2026-03"));
        assert_eq!(s.after.as_deref(), Some("2025-01-15"));
        assert!(s.attachment && s.unread && s.starred);
        let en = parse_search("from:bob has:attachment is:unread is:starred after:2026");
        assert_eq!(
            (en.from.as_slice(), en.attachment, en.unread, en.starred),
            (&["bob".to_string()][..], true, true, true)
        );
        assert_eq!(en.after.as_deref(), Some("2026"));
    }

    #[test]
    fn ce_qui_n_est_pas_un_operateur_reste_du_texte() {
        // Date invalide, opérateur inconnu, valeur vide : tout reste du texte libre.
        let s = parse_search("avant:demain foo:bar de: pj:oui 12:30");
        assert_eq!(
            s,
            SearchSpec {
                text: s.text.clone(),
                ..Default::default()
            }
        );
        assert_eq!(s.text, "avant:demain foo:bar de: pj:oui 12:30");
        assert_eq!(parse_search("   ").text, "");
    }

    #[test]
    fn like_neutralise_les_jokers() {
        assert_eq!(like_contains("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }

    #[test]
    fn dossiers() {
        let s = parse_search(r#"dans:archives budget dans:"Mes Projets" in:tout"#);
        assert_eq!(s.folders, ["archives", "Mes Projets"]);
        assert!(s.anywhere);
        assert_eq!(folder_role("Envoyés"), Some("sent"));
        assert_eq!(folder_role("Corbeille"), Some("trash"));
        assert_eq!(folder_role("clients"), None);
    }

    #[test]
    fn dates() {
        for ok in ["2026", "2026-03", "2026-03-31"] {
            assert!(valid_date(ok), "{ok}");
        }
        for bad in [
            "26",
            "2026-13",
            "2026-03-32",
            "2026-3",
            "x",
            "2026-03-01-02",
        ] {
            assert!(!valid_date(bad), "{bad}");
        }
    }
}
