//! Détection automatique des serveurs d'un compte, dans l'esprit de Thunderbird.
//!
//! Ordre : table intégrée → autoconfig publié par le domaine → base ISPDB de Mozilla →
//! enregistrement MX (fournisseur qui héberge le domaine) → hypothèse `imap.<domaine>`.
//! Les fichiers `config-v1.1.xml` sont une entrée non fiable : hôtes, ports et domaine validés.
//! Vie privée : l'interrogation d'ISPDB révèle le domaine (pas l'adresse) à Mozilla, comme Thunderbird.

use std::time::Duration;

use crate::error::{AppError, Result};
use crate::mail::account::{self, Discovered, Security, ServerConfig};
use crate::mail::oauth::OAuthProvider;

const ISPDB: &str = "https://autoconfig.thunderbird.net/v1.1";
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BODY: usize = 256 * 1024;

fn http() -> Result<&'static reqwest::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let client = crate::tls::http_builder()?
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()?;
    Ok(CLIENT.get_or_init(|| client))
}

/// Détecte les serveurs pour `email` (le domaine est validé avant toute requête réseau).
pub async fn detect(email: &str) -> Result<Discovered> {
    let email = email.trim();
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .filter(|d| valid_hostname(d) && d.contains('.'))
        .ok_or_else(|| {
            AppError::Invalid(msg!(
                "address.invalid",
                "invalid address: {email}",
                email = email
            ))
        })?;

    if let Some(found) = builtin(&domain, email, "builtin") {
        return Ok(found);
    }
    // Autoconfig du domaine (deux emplacements usuels) et ISPDB, en parallèle ; priorité à l'ordre.
    let urls = [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        format!("{ISPDB}/{domain}"),
    ];
    let (a, b, c) = tokio::join!(
        fetch_config(&urls[0], true, &domain, email),
        fetch_config(&urls[1], true, &domain, email),
        fetch_config(&urls[2], false, &domain, email),
    );
    if let Some(d) = a.or(b) {
        return Ok(with_source(d, "autoconfig"));
    }
    if let Some(d) = c {
        return Ok(with_source(d, "ispdb"));
    }
    // Domaine personnalisé hébergé chez un fournisseur : on regarde où pointe le MX.
    if let Some(mx_domain) = mx_registrable_domain(&domain).await {
        if let Some(found) = builtin(&mx_domain, email, "mx") {
            return Ok(found);
        }
        if let Some(d) =
            fetch_config(&format!("{ISPDB}/{mx_domain}"), false, &mx_domain, email).await
        {
            return Ok(with_source(d, "mx"));
        }
    }
    Ok(account::guess(&domain, email))
}

fn with_source(mut d: Discovered, source: &'static str) -> Discovered {
    d.source = source;
    d
}

fn builtin(domain: &str, email: &str, source: &'static str) -> Option<Discovered> {
    let (provider, imap, smtp, oauth) = account::known_provider(domain)?;
    Some(Discovered {
        provider: provider.into(),
        imap,
        smtp,
        username: email.into(),
        oauth,
        source,
    })
}

async fn fetch_config(
    url: &str,
    send_email: bool,
    domain: &str,
    email: &str,
) -> Option<Discovered> {
    let client = match http() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("server auto-detection unavailable: {e}");
            return None;
        }
    };
    let mut req = client.get(url);
    if send_email {
        req = req.query(&[("emailaddress", email)]);
    }
    let resp = req.send().await.ok()?.error_for_status().ok()?;
    if resp.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return None;
    }
    let body = resp.bytes().await.ok()?;
    if body.len() > MAX_BODY {
        return None;
    }
    parse_config(std::str::from_utf8(&body).ok()?, domain, email)
}

/// Lit un `config-v1.1.xml` : meilleur serveur IMAP et SMTP (SSL > STARTTLS > clair).
pub fn parse_config(xml: &str, domain: &str, email: &str) -> Option<Discovered> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let provider = doc
        .descendants()
        .find(|n| n.has_tag_name("emailProvider"))?;
    let local = email.rsplit_once('@').map_or(email, |(l, _)| l);

    let pick = |kind: &str| -> Option<(ServerConfig, String, bool)> {
        provider
            .children()
            .filter(|n| {
                (n.has_tag_name("incomingServer") || n.has_tag_name("outgoingServer"))
                    && n.attribute("type") == Some(kind)
            })
            .filter_map(|n| {
                let text = |tag: &str| {
                    n.children()
                        .find(|c| c.has_tag_name(tag))
                        .and_then(|c| c.text())
                        .map(str::trim)
                };
                let host = text("hostname").filter(|h| valid_hostname(h))?;
                let port: u16 = text("port")?.parse().ok().filter(|p| *p != 0)?;
                let security = match text("socketType")? {
                    "SSL" => Security::Tls,
                    "STARTTLS" => Security::Starttls,
                    "plain" => Security::None,
                    _ => return None,
                };
                let username = text("username")
                    .unwrap_or("%EMAILADDRESS%")
                    .replace("%EMAILADDRESS%", email)
                    .replace("%EMAILLOCALPART%", local)
                    .replace("%EMAILDOMAIN%", domain);
                let oauth = text("authentication") == Some("OAuth2");
                Some((
                    ServerConfig {
                        host: host.to_lowercase(),
                        port,
                        security,
                    },
                    username,
                    oauth,
                ))
            })
            .max_by_key(|(s, _, _)| match s.security {
                Security::Tls => 2,
                Security::Starttls => 1,
                Security::None => 0,
            })
    };
    let (imap, username, imap_oauth) = pick("imap")?;
    let (smtp, _, _) = pick("smtp")?;
    // OAuth2 proposé seulement pour les fournisseurs dont nous savons faire le flux.
    let oauth = imap_oauth
        .then(|| oauth_provider_for_host(&imap.host))
        .flatten();
    Some(Discovered {
        provider: "imap".into(),
        imap,
        smtp,
        username,
        oauth,
        source: "autoconfig",
    })
}

fn oauth_provider_for_host(host: &str) -> Option<OAuthProvider> {
    if host.ends_with("gmail.com") || host.ends_with("google.com") {
        Some(OAuthProvider::Google)
    } else if host.ends_with("office365.com") || host.ends_with("outlook.com") {
        Some(OAuthProvider::Microsoft)
    } else {
        None
    }
}

/// Résolveur système ; à défaut (configuration illisible, p. ex. adresse IPv6 à zone sous macOS),
/// Cloudflare : le domaine (pas l'adresse) est alors visible de ce résolveur public.
fn resolver() -> Option<hickory_resolver::TokioResolver> {
    use hickory_resolver::config::{ResolverConfig, CLOUDFLARE};
    use hickory_resolver::net::runtime::TokioRuntimeProvider;
    use hickory_resolver::TokioResolver;
    match TokioResolver::builder_tokio() {
        Ok(builder) => builder,
        Err(_) => TokioResolver::builder_with_config(
            ResolverConfig::udp_and_tcp(&CLOUDFLARE),
            TokioRuntimeProvider::default(),
        ),
    }
    .build()
    .ok()
}

/// Domaine du premier MX (par préférence), réduit à son domaine enregistrable.
async fn mx_registrable_domain(domain: &str) -> Option<String> {
    let resolver = resolver()?;
    let lookup = tokio::time::timeout(FETCH_TIMEOUT, resolver.mx_lookup(domain))
        .await
        .ok()?
        .ok()?;
    let best = lookup.answers().iter().min_by_key(|r| match &r.data {
        hickory_resolver::proto::rr::RData::MX(mx) => mx.preference,
        _ => u16::MAX,
    })?;
    let hickory_resolver::proto::rr::RData::MX(mx) = &best.data else {
        return None;
    };
    let host = mx.exchange.to_utf8();
    let host = host.trim_end_matches('.').to_lowercase();
    Some(registrable_domain(&host)).filter(|d| valid_hostname(d))
}

/// Domaine enregistrable (« mx1.mail.ovh.net » → « ovh.net », « mail.example.co.uk » → « example.co.uk »),
/// d'après la liste des suffixes publics ; l'hôte lui-même si aucun suffixe connu ne correspond.
pub fn registrable_domain(host: &str) -> String {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    psl::domain_str(&host).unwrap_or(&host).to_string()
}

/// Nom d'hôte DNS plausible (évite l'injection dans les URL et les cibles de connexion).
fn valid_hostname(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1"><emailProvider id="example.org">
  <incomingServer type="imap"><hostname>imap.example.org</hostname><port>143</port>
    <socketType>STARTTLS</socketType><username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication></incomingServer>
  <incomingServer type="imap"><hostname>IMAP.example.org</hostname><port>993</port>
    <socketType>SSL</socketType><username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication></incomingServer>
  <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
  <outgoingServer type="smtp"><hostname>smtp.example.org</hostname><port>587</port>
    <socketType>STARTTLS</socketType><username>%EMAILADDRESS%</username></outgoingServer>
</emailProvider></clientConfig>"#;

    #[test]
    fn parses_config_and_prefers_ssl() {
        let d = parse_config(XML, "example.org", "alice@example.org").expect("config");
        assert_eq!(d.imap.host, "imap.example.org");
        assert_eq!(d.imap.port, 993);
        assert_eq!(d.imap.security, Security::Tls);
        assert_eq!(d.smtp.port, 587);
        assert_eq!(d.smtp.security, Security::Starttls);
        assert_eq!(d.username, "alice");
        assert!(d.oauth.is_none());
    }

    #[test]
    fn rejects_hostile_or_incomplete_config() {
        let bad = XML
            .replace("imap.example.org", "evil.com/../x")
            .replace("IMAP.example.org", "a b");
        assert!(parse_config(&bad, "example.org", "a@example.org").is_none());
        assert!(parse_config("<nope/>", "example.org", "a@example.org").is_none());
        assert!(parse_config("pas du xml", "example.org", "a@example.org").is_none());
    }

    #[test]
    fn oauth_flag_only_for_known_providers() {
        let xml = XML
            .replace("password-cleartext", "OAuth2")
            .replace("imap.example.org", "imap.gmail.com")
            .replace("IMAP.example.org", "imap.gmail.com");
        let d = parse_config(&xml, "example.org", "a@example.org").expect("config");
        assert_eq!(d.oauth, Some(OAuthProvider::Google));
    }

    #[test]
    fn registrable_domains() {
        assert_eq!(registrable_domain("aspmx.l.google.com"), "google.com");
        assert_eq!(registrable_domain("mx1.mail.ovh.net"), "ovh.net");
        assert_eq!(registrable_domain("mail.example.co.uk"), "example.co.uk");
        assert_eq!(registrable_domain("MX.Mail.Example.COM."), "example.com");
        // Ce que l'ancienne approximation (deux ou trois derniers libellés) se trompait à reconnaître :
        assert_eq!(registrable_domain("smtp.example.com.au"), "example.com.au");
        assert_eq!(
            registrable_domain("imap.exemple.gouv.fr"),
            "exemple.gouv.fr"
        );
        assert_eq!(registrable_domain("a.b.exemple.asso.fr"), "exemple.asso.fr");
        assert_eq!(registrable_domain("localhost"), "localhost");
    }

    #[test]
    fn mx_of_hosted_domains_resolves_to_builtin() {
        let d = builtin(
            &registrable_domain("mta-gw.infomaniak.ch"),
            "a@example.net",
            "mx",
        )
        .expect("infomaniak");
        assert_eq!(d.imap.host, "mail.infomaniak.com");
        assert_eq!(d.username, "a@example.net");
        assert!(builtin(&registrable_domain("mx1.mail.ovh.net"), "a@x.fr", "mx").is_some());
    }

    #[test]
    fn hostname_validation() {
        assert!(valid_hostname("imap.example.org"));
        assert!(!valid_hostname("a/b.com"));
        assert!(!valid_hostname("-x.com"));
        assert!(!valid_hostname(""));
    }

    /// Vérification manuelle sur le réseau : `cargo test -p envelope -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn live_detection() {
        for email in [
            "a@mailbox.org",
            "a@gmx.fr",
            "a@laposte.net",
            "a@zoho.com",
            "a@perdu-introuvable-xyz.fr",
        ] {
            match detect(email).await {
                Ok(d) => println!(
                    "{email:32} {:10} {}:{} {:?} / {}:{} {:?} user={} oauth={:?}",
                    d.source,
                    d.imap.host,
                    d.imap.port,
                    d.imap.security,
                    d.smtp.host,
                    d.smtp.port,
                    d.smtp.security,
                    d.username,
                    d.oauth
                ),
                Err(e) => println!("{email}: ERREUR {e}"),
            }
        }
    }
}
