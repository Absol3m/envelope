//! Pièces jointes que l'interface peut afficher elle-même (aperçu et affichage dans le fil).

/// Types d'image rendus par `<img>` (un SVG ainsi chargé n'exécute aucun script).
const PREVIEW_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "image/svg+xml",
];

/// Au-delà, le fichier s'ouvre dans l'application du système.
const PREVIEW_MAX: u64 = 15 * 1024 * 1024;

/// Faut-il afficher cette pièce jointe dans l'application (une seule règle, pour l'interface et pour le protocole
/// `attachment://` qui sert les octets) ?
pub fn previewable(mime: &str, size: u64) -> bool {
    size <= PREVIEW_MAX && PREVIEW_TYPES.iter().any(|t| t.eq_ignore_ascii_case(mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_et_taille() {
        assert!(previewable("image/png", 1));
        assert!(previewable("IMAGE/JPEG", PREVIEW_MAX));
        assert!(previewable("image/svg+xml", 10));
        assert!(!previewable("image/png", PREVIEW_MAX + 1));
        assert!(!previewable("application/pdf", 10));
        assert!(!previewable("image/tiff", 10));
        assert!(!previewable("text/html", 10));
    }
}
