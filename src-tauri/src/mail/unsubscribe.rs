//! Désabonnement des listes de diffusion (List-Unsubscribe, RFC 2369 et RFC 8058).
//!
//! Les en-têtes viennent du message : entrée non fiable. On n'accepte que des liens `https` vers
//! un nom de domaine public (pas d'IP littérale, pas de `localhost`), pour qu'un message piégé ne
//! puisse pas faire émettre de requêtes vers le réseau local de l'utilisateur.

use std::time::Duration;

use envelope_core::thread::Unsubscribe;
use reqwest::redirect::Policy;
use reqwest::Url;

use crate::error::{AppError, Result};
use crate::mail::account::{Account, Credential};
use crate::mail::smtp::{self, OutgoingMessage};

/// Lien https vers un nom de domaine public : seule forme que l'on ouvre ou appelle.
pub fn is_public_https(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d.contains('.')
                && d != "localhost"
                && !d.ends_with(".localhost")
                && !d.ends_with(".local")
                && !d.ends_with(".internal")
        }
        _ => false,
    }
}

/// Extrait le lien https et l'adresse mailto de `List-Unsubscribe` (`<url>, <mailto:...>`).
/// `post` : valeur de `List-Unsubscribe-Post`.
pub fn parse(list: &str, post: Option<&str>) -> Option<Unsubscribe> {
    let mut u = Unsubscribe::default();
    for part in list.split('<').skip(1) {
        let Some((target, _)) = part.split_once('>') else {
            continue;
        };
        let target = target.trim();
        if target.len() > 2048 {
            continue;
        }
        if target.len() >= 7 && target[..7].eq_ignore_ascii_case("mailto:") {
            u.mailto.get_or_insert_with(|| target.to_string());
        } else if Url::parse(target).is_ok_and(|url| is_public_https(&url)) {
            u.url.get_or_insert_with(|| target.to_string());
        }
    }
    u.one_click = u.url.is_some()
        && post.is_some_and(|p| {
            p.to_ascii_lowercase()
                .contains("list-unsubscribe=one-click")
        });
    (u.url.is_some() || u.mailto.is_some()).then_some(u)
}

/// Désabonnement automatique : POST RFC 8058 s'il est proposé, sinon message à l'adresse mailto.
pub async fn run(account: &Account, cred: &Credential, u: &Unsubscribe) -> Result<()> {
    if let (true, Some(url)) = (u.one_click, &u.url) {
        return post_one_click(url).await;
    }
    match &u.mailto {
        Some(m) => send_mailto(account, cred, m).await,
        None => Err(AppError::Invalid(
            "ce message ne propose pas de désabonnement automatique".into(),
        )),
    }
}

async fn post_one_click(url: &str) -> Result<()> {
    let url = Url::parse(url).map_err(|e| AppError::Invalid(e.to_string()))?;
    if !is_public_https(&url) {
        return Err(AppError::Invalid("lien de désabonnement refusé".into()));
    }
    // Redirections suivies seulement vers d'autres liens publics en https.
    let policy = Policy::custom(|a| {
        if a.previous().len() < 3 && is_public_https(a.url()) {
            a.follow()
        } else {
            a.stop()
        }
    });
    let client = crate::tls::http_builder()?
        .redirect(policy)
        .timeout(Duration::from_secs(15))
        .build()?;
    let res = client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("List-Unsubscribe=One-Click")
        .send()
        .await?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "le serveur a refusé le désabonnement ({})",
            res.status()
        )))
    }
}

async fn send_mailto(account: &Account, cred: &Credential, mailto: &str) -> Result<()> {
    let url = Url::parse(mailto).map_err(|e| AppError::Invalid(e.to_string()))?;
    let to = percent_encoding::percent_decode_str(url.path())
        .decode_utf8_lossy()
        .trim()
        .to_string();
    let query = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.replace(['\r', '\n'], " "))
    };
    let subject = query("subject").unwrap_or_else(|| "unsubscribe".into());
    let body = query("body").unwrap_or_else(|| "unsubscribe".into());
    let out = OutgoingMessage {
        to: vec![to],
        cc: vec![],
        bcc: vec![],
        subject,
        body_text: body,
        body_html: None,
        in_reply_to: None,
        references: vec![],
        attachments: vec![],
        draft_id: None,
    };
    smtp::send(account, cred, &out).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrait_https_et_mailto() {
        let u = parse(
            "<mailto:unsub@lists.example.com?subject=stop>, <https://lists.example.com/u/42>",
            Some("List-Unsubscribe=One-Click"),
        )
        .unwrap();
        assert_eq!(u.url.as_deref(), Some("https://lists.example.com/u/42"));
        assert_eq!(
            u.mailto.as_deref(),
            Some("mailto:unsub@lists.example.com?subject=stop")
        );
        assert!(u.one_click);
    }

    #[test]
    fn refuse_http_ip_et_reseau_local() {
        for bad in [
            "<http://example.com/u>",
            "<https://192.168.1.1/u>",
            "<https://[::1]/u>",
            "<https://localhost/u>",
            "<https://nas.local/u>",
            "<https://intranet/u>",
            "<javascript:alert(1)>",
        ] {
            assert!(
                parse(bad, Some("List-Unsubscribe=One-Click")).is_none(),
                "{bad}"
            );
        }
    }

    #[test]
    fn un_clic_exige_l_en_tete_post_et_un_lien() {
        assert!(!parse("<https://a.example.com/u>", None).unwrap().one_click);
        assert!(
            !parse("<mailto:a@b.fr>", Some("List-Unsubscribe=One-Click"))
                .unwrap()
                .one_click
        );
    }
}
