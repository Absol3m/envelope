//! Vérification d'identité de l'expéditeur (SPF, DKIM, DMARC) lue dans l'en-tête `Authentication-Results` (RFC 8601).
//!
//! Aucune bibliothèque de lecture de cet en-tête n'existe (les crates d'authentification sachant le *produire* ne
//! le *lisent* pas) : analyseur borné et testé. Seule la première occurrence de l'en-tête est lue : chaque serveur
//! relais ajoute la sienne en tête, donc c'est celle de **notre** serveur de réception. Un expéditeur peut en
//! forger une plus bas dans le message ; c'est pourquoi on ne retient que les **échecs** : un faux « échec » ne
//! trompe personne, un faux « réussi » le ferait.

/// Mécanisme dont l'échec doit alerter : DMARC d'abord (il synthétise SPF et DKIM avec l'alignement du domaine
/// « From »), sinon SPF ou DKIM s'il n'y a pas de résultat DMARC. `None` : rien à signaler.
pub fn failure(header: &str) -> Option<&'static str> {
    let cleaned = strip_comments(header);
    let (mut dmarc, mut spf, mut dkim) = (None, None, None);
    // Le premier segment est l'identifiant du serveur (« authserv-id »), pas un résultat.
    for part in cleaned.split(';').skip(1).take(32) {
        let part = part.trim_start();
        let Some((method, rest)) = part.split_once('=') else {
            continue;
        };
        let result = rest
            .split(|c: char| c.is_whitespace() || c == ';')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let slot = match method.trim().to_ascii_lowercase().as_str() {
            "dmarc" => &mut dmarc,
            "spf" => &mut spf,
            "dkim" => &mut dkim,
            _ => continue,
        };
        // Plusieurs signatures DKIM : une seule réussie suffit (un relais peut en casser une autre).
        if slot.is_none() || result == "pass" {
            *slot = Some(result);
        }
    }
    let failed = |r: &Option<String>| r.as_deref() == Some("fail");
    if dmarc.is_some() {
        return failed(&dmarc).then_some("dmarc");
    }
    if failed(&spf) {
        return Some("spf");
    }
    failed(&dkim).then_some("dkim")
}

/// Retire les commentaires entre parenthèses (RFC 5322), imbriqués ou non, sans jamais boucler sans fin.
fn strip_comments(s: &str) -> String {
    let (mut out, mut depth) = (String::with_capacity(s.len()), 0u32);
    for c in s.chars().take(8192) {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmarc_en_echec_alerte() {
        let h = "mx.example.net; dkim=pass header.d=bank.fr; spf=pass smtp.mailfrom=bank.fr; dmarc=fail (p=reject) header.from=bank.fr";
        assert_eq!(failure(h), Some("dmarc"));
    }

    #[test]
    fn tout_est_passe_ou_inconnu_rien_a_signaler() {
        assert_eq!(
            failure("mx.example.net; dkim=pass; spf=pass; dmarc=pass"),
            None
        );
        assert_eq!(failure("mx.example.net; none"), None);
        assert_eq!(failure(""), None);
        assert_eq!(failure("mx.example.net; spf=softfail; dkim=neutral"), None);
    }

    #[test]
    fn dmarc_reussi_l_emporte_sur_un_spf_ou_dkim_en_echec() {
        // Transfert ou liste de diffusion : SPF échoue chez le relais mais DMARC passe grâce à DKIM.
        assert_eq!(failure("mx; spf=fail; dkim=pass; dmarc=pass"), None);
    }

    #[test]
    fn sans_dmarc_spf_puis_dkim() {
        assert_eq!(
            failure("mx; spf=fail smtp.mailfrom=x.fr; dkim=pass"),
            Some("spf")
        );
        assert_eq!(failure("mx; dkim=fail header.d=x.fr"), Some("dkim"));
        // Deux signatures DKIM : une réussie suffit.
        assert_eq!(
            failure("mx; dkim=fail header.d=a.fr; dkim=pass header.d=b.fr"),
            None
        );
    }

    #[test]
    fn casse_commentaires_et_entree_hostile() {
        assert_eq!(failure("MX; DMARC=FAIL"), Some("dmarc"));
        // Le mot « fail » dans un commentaire ou dans le nom d'hôte n'est pas un résultat.
        assert_eq!(failure("mx (dmarc=fail); dmarc=pass"), None);
        assert_eq!(failure("fail.example; dmarc=pass"), None);
        // Parenthèses non fermées, énormes ou imbriquées : pas de panique.
        assert_eq!(
            failure(&format!("mx; {}dmarc=fail", "(".repeat(100_000))),
            None
        );
        assert_eq!(failure("mx; ((((dmarc=fail"), None);
        assert_eq!(failure(&"a;".repeat(100_000)), None);
        assert_eq!(failure("\u{0}\u{ffff}; =; =fail; dmarc"), None);
    }
}
