//! S/MIME en lecture : vérification des signatures (CMS, RFC 5652 / 8551) des messages `multipart/signed` et
//! `application/pkcs7-mime`. Même esprit que Thunderbird (NSS) : la signature, puis la chaîne de confiance pour l'usage
//! « e-mail », puis la concordance entre l'adresse du certificat et `From`, enfin l'écart entre la date de signature et
//! `Date:`. Le contenu éventuellement extrait n'est **jamais** écrit en base ni indexé.
//!
//! Pas de cryptographie maison : `cms` + `x509-cert` lisent les structures (sans leurs options RSA, donc sans la
//! crate `rsa` et ses avis), `webpki` vérifie la signature et la chaîne, et les autorités viennent du magasin du système
//! (`rustls-native-certs`) : jamais de liste embarquée (règle du projet, voir `tls.rs`).

use cms::cert::CertificateChoices;
use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier};
use der::asn1::{GeneralizedTime, Ia5String, ObjectIdentifier, OctetString, UtcTime};
use der::{Decode, Encode};
use mail_parser::{MessageParser, MimeHeaders};
use rustls::pki_types::{CertificateDer, SignatureVerificationAlgorithm, TrustAnchor, UnixTime};
use serde::Serialize;
use sha2::{Digest, Sha256, Sha384, Sha512};
use webpki::{EndEntityCert, KeyUsage};
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::SubjectAltName;
use x509_cert::Certificate;

/// Taille maximale d'une structure CMS lue : borne contre un message forgé.
const MAX_CMS: usize = 32 * 1024 * 1024;
/// Écart toléré entre la date de signature et `Date:` (comme Thunderbird : une heure).
const MAX_SKEW_SECS: i64 = 3600;

const SIGNED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
const ENVELOPED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.3");
const ATTR_MESSAGE_DIGEST: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
const ATTR_SIGNING_TIME: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
const ATTR_EMAIL: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");
const SAN: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.17");
/// Usage `emailProtection` (1.3.6.1.5.5.7.3.4) : contenu DER de l'OID, tel que webpki l'attend.
const EMAIL_PROTECTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");
const RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const RSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11");
const RSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.12");
const RSA_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.13");
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SmimeState {
    /// Signature correcte, chaîne de confiance valide pour l'e-mail, adresse du certificat = expéditeur.
    Valid,
    /// La signature ne correspond pas au contenu (message altéré) ou la structure est illisible.
    Invalid,
    /// Signature correcte mais le certificat ne remonte à aucune autorité de confiance du système (ou n'est pas valable pour l'e-mail).
    Untrusted,
    /// Signature et chaîne correctes, mais l'adresse du certificat n'est pas celle de `From`.
    AddressMismatch,
    /// Le certificat ne contient aucune adresse e-mail : on ne peut pas le rattacher à l'expéditeur.
    NoAddress,
    /// Signature et adresse correctes, mais la date de signature s'écarte de plus d'une heure de `Date:`.
    TimeMismatch,
    /// Algorithme de signature non pris en charge (RSA-PSS, SHA-1, clé RSA < 2048 bits…) : impossible de conclure.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SmimeSignature {
    pub state: SmimeState,
    /// Adresse (ou, à défaut, sujet) du certificat de signature.
    pub signer: Option<String>,
}

impl SmimeSignature {
    fn of(state: SmimeState, signer: Option<String>) -> Self {
        Self { state, signer }
    }
}

/// Forme S/MIME reconnue dans le message brut.
#[derive(Debug, PartialEq, Eq)]
pub enum Shape {
    /// `multipart/signed` : octets exacts du contenu signé (fins de ligne CRLF) et signature détachée (DER).
    Signed {
        content: Vec<u8>,
        signature: Vec<u8>,
    },
    /// `application/pkcs7-mime` signé : le contenu est dans la structure CMS.
    Opaque { cms: Vec<u8> },
    /// `application/pkcs7-mime` chiffré (enveloped-data) : pas encore pris en charge.
    Encrypted,
}

type Alg = &'static dyn SignatureVerificationAlgorithm;

fn is_pkcs7(subtype: &str) -> bool {
    matches!(subtype, "pkcs7-mime" | "x-pkcs7-mime")
}

/// Reconnaît la forme S/MIME d'un message brut. `None` : pas de S/MIME.
pub fn shape(raw: &[u8]) -> Option<Shape> {
    let msg = MessageParser::default().parse(raw)?;
    let root = msg.root_part();
    let ct = root.content_type()?;
    let subtype = ct.subtype().unwrap_or("").to_ascii_lowercase();
    if ct.ctype().eq_ignore_ascii_case("multipart") {
        let protocol = ct.attribute("protocol").unwrap_or("").to_ascii_lowercase();
        let kids = root.sub_parts()?;
        let pkcs7 = matches!(
            protocol.as_str(),
            "application/pkcs7-signature" | "application/x-pkcs7-signature"
        );
        if subtype != "signed" || !pkcs7 || kids.len() < 2 {
            return None;
        }
        let first = msg.part(kids[0])?;
        let sig = msg.part(kids[1])?;
        let content =
            raw.get(first.raw_header_offset() as usize..first.raw_end_offset() as usize)?;
        return Some(Shape::Signed {
            content: crate::mail::mime_util::crlf(content),
            signature: sig.contents().to_vec(),
        });
    }
    if !ct.ctype().eq_ignore_ascii_case("application") || !is_pkcs7(&subtype) {
        return None;
    }
    let der = root.contents();
    if der.len() > MAX_CMS {
        return None;
    }
    // `smime-type` est facultatif : le type du contenu CMS tranche.
    let info = ContentInfo::from_der(der).ok()?;
    if info.content_type == SIGNED_DATA {
        Some(Shape::Opaque { cms: der.to_vec() })
    } else if info.content_type == ENVELOPED_DATA {
        Some(Shape::Encrypted)
    } else {
        None
    }
}

/// Autorités de confiance du système, chargées une fois (une autorité ajoutée ou retirée ensuite n'est vue qu'au
/// redémarrage). Pourquoi pas `rustls-platform-verifier` (`tls.rs`) : il ne sait vérifier que l'usage « serveur TLS » ;
/// l'usage « e-mail » (EKU `emailProtection`) exige webpki, qui demande les ancres elles-mêmes. Limites connues :
/// pas de contrôle de révocation (CRL/OCSP), suivi dans #130. Une autorité illisible est ignorée.
pub fn system_roots() -> &'static [TrustAnchor<'static>] {
    static ROOTS: std::sync::OnceLock<Vec<TrustAnchor<'static>>> = std::sync::OnceLock::new();
    ROOTS.get_or_init(|| {
        rustls_native_certs::load_native_certs()
            .certs
            .iter()
            .filter_map(|c| {
                webpki::anchor_from_trusted_cert(c)
                    .ok()
                    .map(|a| a.to_owned())
            })
            .collect()
    })
}

/// Ce que l'on sait du message : expéditeur(s) de `From` et date de `Date:`.
pub struct Envelope {
    pub from: Vec<String>,
    pub date: Option<i64>,
}

impl Envelope {
    pub fn of(raw: &[u8]) -> Self {
        let msg = MessageParser::default().parse(raw);
        let from = msg
            .as_ref()
            .and_then(|m| m.from())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.address())
                    .map(str::to_lowercase)
                    .collect()
            })
            .unwrap_or_default();
        let date = msg
            .as_ref()
            .and_then(|m| m.date())
            .map(|d| d.to_timestamp());
        Self { from, date }
    }
}

/// Résultat de la vérification : verdict, et contenu extrait d'une signature opaque.
pub struct Verified {
    pub signature: SmimeSignature,
    pub content: Option<Vec<u8>>,
}

fn invalid() -> Verified {
    Verified {
        signature: SmimeSignature::of(SmimeState::Invalid, None),
        content: None,
    }
}

/// Vérifie une structure CMS signée. `detached` : contenu d'une signature détachée (`multipart/signed`).
pub fn verify(
    cms_der: &[u8],
    detached: Option<&[u8]>,
    envelope: &Envelope,
    anchors: &[TrustAnchor<'_>],
    now: UnixTime,
) -> Verified {
    verify_inner(cms_der, detached, envelope, anchors, now).unwrap_or_else(invalid)
}

fn verify_inner(
    cms_der: &[u8],
    detached: Option<&[u8]>,
    envelope: &Envelope,
    anchors: &[TrustAnchor<'_>],
    now: UnixTime,
) -> Option<Verified> {
    if cms_der.len() > MAX_CMS {
        return None;
    }
    let info = ContentInfo::from_der(cms_der).ok()?;
    if info.content_type != SIGNED_DATA {
        return None;
    }
    let sd: SignedData = info.content.decode_as().ok()?;
    let signer_info = sd.signer_infos.0.iter().next()?;

    let certs: Vec<Certificate> = sd
        .certificates
        .as_ref()?
        .0
        .iter()
        .filter_map(|c| match c {
            CertificateChoices::Certificate(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    let signer = certs.iter().find(|c| match &signer_info.sid {
        SignerIdentifier::IssuerAndSerialNumber(i) => {
            c.tbs_certificate.issuer == i.issuer
                && c.tbs_certificate.serial_number == i.serial_number
        }
        SignerIdentifier::SubjectKeyIdentifier(_) => false,
    })?;
    let signer_der = signer.to_der().ok()?;

    let embedded: Option<Vec<u8>> = sd
        .encap_content_info
        .econtent
        .as_ref()
        .and_then(|any| any.decode_as::<OctetString>().ok())
        .map(|o| o.as_bytes().to_vec());
    let content: &[u8] = detached.or(embedded.as_deref())?;

    let names = cert_emails(signer);
    let shown = names
        .first()
        .cloned()
        .or_else(|| Some(signer.tbs_certificate.subject.to_string()));
    let verdict = |state| {
        Some(Verified {
            signature: SmimeSignature::of(state, shown.clone()),
            content: embedded.clone().filter(|_| detached.is_none()),
        })
    };

    // Condensat du contenu, puis signature : sur les attributs signés s'il y en a (leur condensat doit être celui du
    // contenu), sinon directement sur le contenu.
    let digest = match digest_of(&signer_info.digest_alg.oid, content) {
        Some(d) => d,
        None => return verdict(SmimeState::Unsupported),
    };
    let to_verify: Vec<u8> = match &signer_info.signed_attrs {
        Some(attrs) => {
            let claimed = attrs
                .iter()
                .find(|a| a.oid == ATTR_MESSAGE_DIGEST)?
                .values
                .iter()
                .next()?
                .decode_as::<OctetString>()
                .ok()?;
            if claimed.as_bytes() != digest.as_slice() {
                return verdict(SmimeState::Invalid);
            }
            attrs.to_der().ok()?
        }
        None => content.to_vec(),
    };
    let leaf = CertificateDer::from(signer_der);
    let ee = EndEntityCert::try_from(&leaf).ok()?;
    let algs = signature_algs(
        &signer_info.signature_algorithm.oid,
        &signer_info.digest_alg.oid,
    );
    if algs.is_empty() {
        return verdict(SmimeState::Unsupported);
    }
    let sig = signer_info.signature.as_bytes();
    if !algs
        .iter()
        .any(|a| ee.verify_signature(*a, &to_verify, sig).is_ok())
    {
        return verdict(SmimeState::Invalid);
    }

    // Chaîne : autres certificats joints comme intermédiaires, usage « e-mail » (ou absence d'usage déclaré).
    let intermediates: Vec<CertificateDer<'static>> = certs
        .iter()
        .filter(|c| *c != signer)
        .filter_map(|c| c.to_der().ok().map(CertificateDer::from))
        .collect();
    if ee
        .verify_for_usage(
            webpki::ALL_VERIFICATION_ALGS,
            anchors,
            &intermediates,
            now,
            KeyUsage::required_if_present(EMAIL_PROTECTION.as_bytes()),
            None,
            None,
        )
        .is_err()
    {
        return verdict(SmimeState::Untrusted);
    }

    if names.is_empty() {
        return verdict(SmimeState::NoAddress);
    }
    if !envelope.from.iter().any(|f| names.contains(f)) {
        return verdict(SmimeState::AddressMismatch);
    }
    // La date de signature est facultative ; si elle est là, elle doit être proche de `Date:`.
    if let (Some(signed), Some(dated)) = (signing_time(signer_info), envelope.date) {
        if (signed - dated).abs() > MAX_SKEW_SECS {
            return verdict(SmimeState::TimeMismatch);
        }
    }
    verdict(SmimeState::Valid)
}

fn digest_of(oid: &ObjectIdentifier, content: &[u8]) -> Option<Vec<u8>> {
    match *oid {
        SHA256 => Some(Sha256::digest(content).to_vec()),
        SHA384 => Some(Sha384::digest(content).to_vec()),
        SHA512 => Some(Sha512::digest(content).to_vec()),
        _ => None, // SHA-1 et MD5 : plus assez sûrs pour conclure
    }
}

/// Algorithmes de vérification à essayer pour cette signature (la courbe d'une clé ECDSA ne se lit pas dans l'OID).
fn signature_algs(signature: &ObjectIdentifier, digest: &ObjectIdentifier) -> Vec<Alg> {
    use webpki::ring::*;
    match *signature {
        // rsaEncryption (le hachage vient du condensat) et sha{256,384,512}WithRSAEncryption
        RSA_ENCRYPTION | RSA_SHA256 | RSA_SHA384 | RSA_SHA512 => match *digest {
            SHA256 => vec![RSA_PKCS1_2048_8192_SHA256],
            SHA384 => vec![RSA_PKCS1_2048_8192_SHA384],
            SHA512 => vec![RSA_PKCS1_2048_8192_SHA512],
            _ => vec![],
        },
        ECDSA_SHA256 => vec![ECDSA_P256_SHA256, ECDSA_P384_SHA256],
        ECDSA_SHA384 => vec![ECDSA_P384_SHA384, ECDSA_P256_SHA384],
        _ => vec![],
    }
}

fn signing_time(si: &cms::signed_data::SignerInfo) -> Option<i64> {
    let value = si
        .signed_attrs
        .as_ref()?
        .iter()
        .find(|a| a.oid == ATTR_SIGNING_TIME)?
        .values
        .iter()
        .next()?;
    let secs = |d: std::time::Duration| i64::try_from(d.as_secs()).ok();
    value
        .decode_as::<UtcTime>()
        .ok()
        .and_then(|t| secs(t.to_unix_duration()))
        .or_else(|| {
            value
                .decode_as::<GeneralizedTime>()
                .ok()
                .and_then(|t| secs(t.to_unix_duration()))
        })
}

/// Adresses du certificat (SAN `rfc822Name`, puis attribut `emailAddress` du sujet), en minuscules.
fn cert_emails(cert: &Certificate) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for ext in cert.tbs_certificate.extensions.iter().flatten() {
        if ext.extn_id != SAN {
            continue;
        }
        if let Ok(san) = SubjectAltName::from_der(ext.extn_value.as_bytes()) {
            for name in san.0 {
                if let GeneralName::Rfc822Name(a) = name {
                    out.push(a.to_string().to_lowercase());
                }
            }
        }
    }
    for rdn in cert.tbs_certificate.subject.0.iter() {
        for atv in rdn.0.iter().filter(|a| a.oid == ATTR_EMAIL) {
            if let Ok(a) = atv.value.decode_as::<Ia5String>() {
                out.push(a.to_string().to_lowercase());
            }
        }
    }
    out.dedup();
    out
}

#[derive(Debug, Clone, Default, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SmimeInspection {
    /// Message chiffré : pas encore pris en charge, on ne peut pas l'ouvrir.
    pub encrypted: bool,
    pub signature: Option<SmimeSignature>,
    /// Contenu d'une signature opaque : à afficher à la place du corps stocké.
    pub body_text: Option<String>,
    pub body_html: Option<String>,
}

/// Analyse un message brut. `None` : pas de S/MIME.
/// `roots` n'est appelé que pour une signature : on ne charge pas les autorités du système pour un message ordinaire.
pub fn analyse<A: AsRef<[TrustAnchor<'static>]>>(
    raw: &[u8],
    roots: impl FnOnce() -> A,
    now: UnixTime,
) -> Option<SmimeInspection> {
    let shape = shape(raw)?;
    let envelope = Envelope::of(raw);
    Some(match shape {
        Shape::Encrypted => SmimeInspection {
            encrypted: true,
            ..Default::default()
        },
        Shape::Signed { content, signature } => SmimeInspection {
            signature: Some(
                verify(&signature, Some(&content), &envelope, roots().as_ref(), now).signature,
            ),
            ..Default::default()
        },
        Shape::Opaque { cms } => {
            let v = verify(&cms, None, &envelope, roots().as_ref(), now);
            let (body_text, body_html) = v
                .content
                .map(|c| crate::mail::mime_util::text_of(&c, true))
                .unwrap_or_default();
            SmimeInspection {
                signature: Some(v.signature),
                body_text,
                body_html,
                ..Default::default()
            }
        }
    })
}

/// Lit la forme S/MIME d'un message de la base (`None` : pas de S/MIME). Le calcul se fait hors de l'exécuteur.
pub async fn inspect(
    pool: &sqlx::SqlitePool,
    raw_dir: &std::path::Path,
    message_id: i64,
) -> crate::error::Result<Option<SmimeInspection>> {
    let Some(raw) = crate::mail::rawstore::of_message(pool, raw_dir, message_id).await? else {
        return Ok(None);
    };
    match tokio::task::spawn_blocking(move || analyse(&raw, system_roots, UnixTime::now())).await {
        Ok(r) => Ok(r),
        Err(e) => {
            tracing::warn!("S/MIME analysis interrupted: {e}");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> UnixTime {
        UnixTime::since_unix_epoch(std::time::Duration::from_secs(1_800_000_000))
    }

    fn fixture(name: &str) -> Vec<u8> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smime");
        std::fs::read(dir.join(name)).unwrap()
    }

    fn ca() -> Vec<TrustAnchor<'static>> {
        let pem = String::from_utf8(fixture("ca.pem")).unwrap();
        let b64: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(&b64)
                .unwrap()
        };
        let cert = CertificateDer::from(der);
        vec![webpki::anchor_from_trusted_cert(&cert).unwrap().to_owned()]
    }

    fn state(raw: &[u8], anchors: &[TrustAnchor<'static>]) -> SmimeState {
        analyse(raw, || anchors.to_vec(), now())
            .and_then(|i| i.signature)
            .map(|s| s.state)
            .unwrap()
    }

    /// L'en-tête `From:` de la fixture (ajouté à la volée : openssl n'en met pas).
    fn with_headers(raw: &[u8], from: &str, date: &str) -> Vec<u8> {
        let mut out = format!("From: {from}\r\nDate: {date}\r\n").into_bytes();
        out.extend_from_slice(raw);
        out
    }

    /// Proche de la date de signature des fixtures (26 nov. 2026, vers 13 h 30 UTC) : écart bien sous l'heure tolérée.
    const DATE: &str = "Thu, 26 Nov 2026 14:00:00 +0000";

    #[test]
    fn signature_rsa_detachee_valide() {
        let raw = with_headers(
            &fixture("rsa_detached.eml"),
            "Alice <alice@example.org>",
            DATE,
        );
        let i = analyse(&raw, ca, now()).unwrap();
        let sig = i.signature.unwrap();
        assert_eq!(sig.state, SmimeState::Valid);
        assert_eq!(sig.signer.as_deref(), Some("alice@example.org"));
    }

    #[test]
    fn autres_algorithmes_et_formes() {
        for name in [
            "ec_detached.eml",
            "rsa_sha384_detached.eml",
            "noeku_detached.eml",
        ] {
            let raw = with_headers(&fixture(name), "alice@example.org", DATE);
            assert_eq!(state(&raw, &ca()), SmimeState::Valid, "{name}");
        }
        // signature opaque : le contenu est extrait pour l'affichage
        let raw = with_headers(&fixture("rsa_opaque.eml"), "alice@example.org", DATE);
        let i = analyse(&raw, ca, now()).unwrap();
        assert_eq!(i.signature.unwrap().state, SmimeState::Valid);
        assert_eq!(i.body_text.as_deref().map(str::trim), Some("Bonjour Bob"));
    }

    #[test]
    fn contenu_altere_est_invalide() {
        let raw = with_headers(&fixture("rsa_detached.eml"), "alice@example.org", DATE);
        let altered = String::from_utf8(raw)
            .unwrap()
            .replace("Bonjour Bob", "Bonjour Bib")
            .into_bytes();
        assert_eq!(state(&altered, &ca()), SmimeState::Invalid);
    }

    #[test]
    fn autorite_inconnue_ou_usage_serveur_est_non_fiable() {
        let raw = with_headers(&fixture("rsa_detached.eml"), "alice@example.org", DATE);
        assert_eq!(state(&raw, &[]), SmimeState::Untrusted);
        let tls = with_headers(&fixture("tls_detached.eml"), "alice@example.org", DATE);
        assert_eq!(state(&tls, &ca()), SmimeState::Untrusted);
    }

    #[test]
    fn adresse_et_date_sont_comparees_a_l_en_tete() {
        let other = with_headers(&fixture("rsa_detached.eml"), "mallory@example.net", DATE);
        assert_eq!(state(&other, &ca()), SmimeState::AddressMismatch);
        // les fixtures sont signées fin 2026 : un message daté de 2020 s'écarte de plus d'une heure
        let old = with_headers(
            &fixture("rsa_detached.eml"),
            "ALICE@example.org",
            "Mon, 06 Jan 2020 10:00:00 +0000",
        );
        assert_eq!(state(&old, &ca()), SmimeState::TimeMismatch);
    }

    #[test]
    fn message_chiffre_ou_sans_smime() {
        let enc = analyse(&fixture("rsa_encrypted.eml"), ca, now()).unwrap();
        assert!(enc.encrypted && enc.signature.is_none());
        assert!(analyse(
            b"From: a@b.c\r\n\r\nbonjour",
            ca,
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(1))
        )
        .is_none());
    }

    #[test]
    fn structure_forgee_ne_panique_pas() {
        let env = Envelope {
            from: vec![],
            date: None,
        };
        let now = UnixTime::since_unix_epoch(std::time::Duration::from_secs(1));
        for junk in [&b""[..], b"\x30\x03\x02\x01\x01", &[0xff; 64]] {
            assert_eq!(
                verify(junk, Some(b"x"), &env, &[], now).signature.state,
                SmimeState::Invalid
            );
        }
    }
}
