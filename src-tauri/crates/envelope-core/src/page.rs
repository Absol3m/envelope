//! Pagination : une seule politique pour l'application et le serveur MCP (borne haute, départ jamais négatif).

/// Fenêtre de résultats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
}

impl Page {
    /// Plus grande fenêtre servie, quoi que demande l'appelant.
    pub const MAX_LIMIT: i64 = 500;

    /// Fenêtre demandée (`None` : valeur par défaut), ramenée dans les bornes.
    pub fn new(limit: Option<i64>, offset: Option<i64>, default_limit: i64) -> Self {
        Page {
            limit: limit.unwrap_or(default_limit).clamp(1, Self::MAX_LIMIT),
            offset: offset.unwrap_or(0).max(0),
        }
    }

    /// Début de liste, `limit` résultats au plus.
    pub fn first(limit: i64) -> Self {
        Self::new(Some(limit), None, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bornes_et_valeurs_par_defaut() {
        assert_eq!(
            Page::new(None, None, 50),
            Page {
                limit: 50,
                offset: 0
            }
        );
        assert_eq!(
            Page::new(Some(0), Some(-5), 50),
            Page {
                limit: 1,
                offset: 0
            }
        );
        assert_eq!(
            Page::new(Some(10_000), Some(40), 50),
            Page {
                limit: 500,
                offset: 40
            }
        );
        assert_eq!(
            Page::first(20),
            Page {
                limit: 20,
                offset: 0
            }
        );
    }
}
