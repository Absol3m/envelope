//! Protocole `attachment://localhost/<id>` : sert à la webview les octets d'une pièce jointe affichable (image).
//!
//! Pas de base64 ni de passage par l'IPC JSON : l'interface met l'URL dans un `<img>`, le moteur de rendu la lit en
//! flux et la libère comme n'importe quelle image. Mécanisme officiel de Tauri (protocole d'URI
//! personnalisé), pas de serveur local. Même règle d'affichage que l'interface (`envelope_core::attachment`), et
//! refus tant que l'application est verrouillée (la barrière des commandes ne couvre pas ce chemin).

use std::borrow::Cow;
use std::path::Path;

use envelope_core::attachment::{is_pdf, previewable};
use sqlx::SqlitePool;
use tauri::http::{header, Response, StatusCode};
use tauri::{AppHandle, Manager};
use tokio::io::AsyncReadExt;

use crate::commands::attachment_file;
use crate::{lock, AppState};

pub const SCHEME: &str = "attachment";

/// L'identifiant de la pièce jointe dans `/<id>` (un entier, rien d'autre).
fn parse_id(path: &str) -> Option<i64> {
    let digits = path.strip_prefix('/')?;
    if digits.is_empty() || digits.len() > 18 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn status(code: StatusCode) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(code)
        .body(Cow::Borrowed(&[][..]))
        .unwrap_or_default()
}

pub async fn serve(app: &AppHandle, path: &str) -> Response<Cow<'static, [u8]>> {
    if lock::is_locked_or_missing(app) {
        return status(StatusCode::FORBIDDEN);
    }
    let Some(id) = parse_id(path) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let state = app.state::<AppState>();
    serve_id(&state.db, &state.attachments_dir, &state.raw_dir, id).await
}

async fn serve_id(
    db: &SqlitePool,
    attachments_dir: &Path,
    raw_dir: &Path,
    id: i64,
) -> Response<Cow<'static, [u8]>> {
    let Ok(file) = attachment_file(db, attachments_dir, raw_dir, id).await else {
        return status(StatusCode::NOT_FOUND);
    };
    // Un seul accès au fichier : la taille vérifiée est celle du fichier lu.
    let Ok(mut f) = tokio::fs::File::open(&file.path).await else {
        return status(StatusCode::NOT_FOUND);
    };
    let Ok(size) = f.metadata().await.map(|m| m.len()) else {
        return status(StatusCode::NOT_FOUND);
    };
    if !previewable(&file.mime, size) {
        return status(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    if f.read_to_end(&mut bytes).await.is_err() {
        return status(StatusCode::NOT_FOUND);
    }
    let mut res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, file.mime.to_ascii_lowercase());
    // Ouvert directement, un SVG n'exécute rien et ne charge rien. Pas de CSP pour un PDF : elle empêcherait le
    // lecteur natif de la webview de s'afficher (le type est figé par nosniff, ce n'est pas du HTML).
    if !is_pdf(&file.mime) {
        res = res.header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'",
        );
    }
    res.header("X-Content-Type-Options", "nosniff")
        // Pas de cache du moteur de rendu : une image déjà chargée ne contournerait plus le verrouillage, et un
        // identifiant réutilisé ne montrerait pas l'ancienne image. L'accès est local, la relecture est peu coûteuse.
        .header(header::CACHE_CONTROL, "no-store")
        .body(Cow::Owned(bytes))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiant_entier_seulement() {
        assert_eq!(parse_id("/42"), Some(42));
        assert_eq!(parse_id("/0"), Some(0));
        for bad in [
            "",
            "/",
            "42",
            "/-1",
            "/4 2",
            "/42/x",
            "/../etc/passwd",
            "/%34",
            "/1e3",
            "/9999999999999999999",
        ] {
            assert_eq!(parse_id(bad), None, "{bad}");
        }
    }

    async fn insert(db: &SqlitePool, mime: &str, size: i64, path: &Path) -> i64 {
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(db)
            .await
            .unwrap();
        sqlx::query_scalar(
            "INSERT INTO attachments (message_id, filename, mime_type, size, storage_path) VALUES (1, 'a', ?, ?, ?) RETURNING id",
        )
        .bind(mime)
        .bind(size)
        .bind(path.to_string_lossy().to_string())
        .fetch_one(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn sert_une_image_et_refuse_le_reste() {
        let db = crate::db::init_memory().await.unwrap();
        let dir = std::env::temp_dir().join(format!("envelope-proto-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("a.png");
        std::fs::write(&img, b"\x89PNG").unwrap();
        let pdf = dir.join("a.pdf");
        std::fs::write(&pdf, b"%PDF").unwrap();
        let outside =
            std::env::temp_dir().join(format!("envelope-out-{}.png", uuid::Uuid::new_v4()));
        std::fs::write(&outside, b"x").unwrap();

        let ok = insert(&db, "image/png", 4, &img).await;
        let r = serve_id(&db, &dir, &dir, ok).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(r.body().as_ref(), b"\x89PNG");

        let doc = insert(&db, "application/pdf", 4, &pdf).await;
        let r = serve_id(&db, &dir, &dir, doc).await;
        assert_eq!(r.status(), StatusCode::OK);
        assert!(r.headers().get(header::CONTENT_SECURITY_POLICY).is_none());
        let txt = dir.join("a.html");
        std::fs::write(&txt, b"<b>").unwrap();
        let not_shown = insert(&db, "text/html", 3, &txt).await;
        assert_eq!(
            serve_id(&db, &dir, &dir, not_shown).await.status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        let escaped = insert(&db, "image/png", 1, &outside).await;
        assert_eq!(
            serve_id(&db, &dir, &dir, escaped).await.status(),
            StatusCode::NOT_FOUND,
            "hors du dossier des pièces jointes"
        );
        assert_eq!(
            serve_id(&db, &dir, &dir, 9999).await.status(),
            StatusCode::NOT_FOUND
        );
    }
}
