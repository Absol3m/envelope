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
            // Un point final (« localhost. », « x.local. ») désigne le même hôte.
            let d = d.trim_end_matches('.').to_ascii_lowercase();
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
        // `get` : une découpe par octets au milieu d'un caractère (cible non ASCII d'un mail hostile) ferait paniquer, et
        // l'application s'arrête sur toute panique (`panic = "abort"`) : à chaque synchro du message.
        if target
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("mailto:"))
        {
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
        None => Err(AppError::Invalid(msg!(
            "unsubscribe.unsupported",
            "this message doesn't offer automatic unsubscription"
        ))),
    }
}

/// Adresses publiques de `host` ; erreur si le nom n'en a aucune (règle commune au résolveur et à la vérification préalable).
async fn resolve_public(host: &str) -> std::io::Result<Vec<std::net::SocketAddr>> {
    let public: Vec<_> = tokio::net::lookup_host((host, 443))
        .await?
        .filter(|a| is_public_ip(a.ip()))
        .collect();
    if public.is_empty() {
        return Err(std::io::Error::other(format!(
            "{host} resolves to a non-public address"
        )));
    }
    Ok(public)
}

/// Résolveur qui ne rend que des adresses publiques : un nom public qui pointerait vers le réseau local, la boucle
/// locale ou les métadonnées d'un fournisseur de cloud (enregistrement DNS d'un mail hostile) n'est pas contacté.
struct PublicOnly;

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move {
            let public = resolve_public(&host).await?;
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Adresse joignable depuis Internet : ni privée, ni boucle locale, ni lien local, ni partagée (CGNAT), ni réservée.
// Écrit à la main : `IpAddr::is_global` n'est pas stable. NAT64 et 6to4 : on juge l'IPv4 qu'ils embarquent.
fn is_public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || (o[0] == 100 && (64..128).contains(&o[1])) // 100.64.0.0/10 (CGNAT)
                || o[0] == 0
                || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24 (protocoles IETF)
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18.0.0/15 (bancs d'essai)
                || o[0] >= 240) // 240.0.0.0/4 (réservé)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(v4));
            }
            let s = v.segments();
            // NAT64 (64:ff9b::/96) et 6to4 (2002::/16) embarquent une adresse IPv4 : c'est elle qu'on juge.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                    (s[6] >> 8) as u8,
                    s[6] as u8,
                    (s[7] >> 8) as u8,
                    s[7] as u8,
                )));
            }
            if s[0] == 0x2002 {
                return is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(
                    (s[1] >> 8) as u8,
                    s[1] as u8,
                    (s[2] >> 8) as u8,
                    s[2] as u8,
                )));
            }
            !(v.is_loopback()
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x2001 && s[1] == 0) // Teredo (2001::/32) : IPv4 masquée
                || v.is_unspecified()
                || v.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // fc00::/7 (adresses locales uniques)
                || (s[0] & 0xffc0) == 0xfe80) // fe80::/10 (lien local)
        }
    }
}

/// Le nom de l'hôte ne doit résoudre que vers des adresses publiques (même règle que `PublicOnly`).
async fn ensure_public_host(url: &Url) -> Result<()> {
    match url.host_str() {
        Some(host) if resolve_public(host).await.is_ok() => Ok(()),
        _ => Err(AppError::Invalid(msg!(
            "unsubscribe.refused_link",
            "unsubscribe link refused"
        ))),
    }
}

async fn post_one_click(url: &str) -> Result<()> {
    let url = Url::parse(url).map_err(|e| AppError::Invalid((e.to_string()).into()))?;
    if !is_public_https(&url) {
        return Err(AppError::Invalid(msg!(
            "unsubscribe.refused_link",
            "unsubscribe link refused"
        )));
    }
    // Derrière un proxy système, c'est le proxy qui résout le nom et `PublicOnly` est contourné : on résout donc aussi
    // ici, avant d'envoyer. (`no_proxy` casserait les réseaux d'entreprise.) Le proxy reste maître des redirections.
    ensure_public_host(&url).await?;
    // Redirections suivies seulement vers d'autres liens publics en https.
    let policy = Policy::custom(|a| {
        if a.previous().len() < 3 && is_public_https(a.url()) {
            a.follow()
        } else {
            a.stop()
        }
    });
    let client = crate::tls::http_builder()?
        .dns_resolver(std::sync::Arc::new(PublicOnly))
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
        Err(AppError::Invalid(msg!(
            "unsubscribe.server_refused",
            "the server refused the unsubscription ({status})",
            status = res.status()
        )))
    }
}

async fn send_mailto(account: &Account, cred: &Credential, mailto: &str) -> Result<()> {
    let url = Url::parse(mailto).map_err(|e| AppError::Invalid((e.to_string()).into()))?;
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
        from_email: None,
        pgp_sign: false,
        pgp_encrypt: false,
    };
    smtp::send(account, cred, &out).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nom_avec_point_final_reste_un_hote_local() {
        for u in [
            "https://localhost./u",
            "https://x.local./u",
            "https://svc.internal./u",
            "https://a.localhost./u",
        ] {
            assert!(!is_public_https(&Url::parse(u).unwrap()), "{u}");
        }
        assert!(is_public_https(
            &Url::parse("https://lists.example.com./u").unwrap()
        ));
    }

    #[test]
    fn seules_les_adresses_publiques_sont_contactees() {
        for ip in [
            "127.0.0.1",
            "10.0.0.5",
            "192.168.1.1",
            "172.16.0.9",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fd00::1",
            "::ffff:127.0.0.1",
            "240.0.0.1",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.254",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2", // Teredo
            "64:ff9b::7f00:1",                      // NAT64 vers 127.0.0.1
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "93.184.216.34",
            "8.8.8.8",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
            "198.20.0.1",
            "192.1.0.1",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    /// Une cible non ASCII dont l'octet 7 tombe au milieu d'un caractère ne fait pas paniquer (l'application
    /// s'arrêtait à chaque synchro du message).
    #[test]
    fn cible_non_ascii_ne_fait_pas_paniquer() {
        for list in [
            "<éééé@x.fr>",
            "<ééééé>",
            "<mailtoé:a>",
            "<€€€€€€€>, <mailto:a@x.fr>",
        ] {
            let _ = parse(list, None);
        }
        assert_eq!(
            parse("<€€€€€€€>, <mailto:a@x.fr>", None).and_then(|u| u.mailto),
            Some("mailto:a@x.fr".into())
        );
    }

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
