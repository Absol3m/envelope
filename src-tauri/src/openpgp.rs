//! OpenPGP en lecture : vérification des signatures et déchiffrement des messages (PGP/MIME, RFC 3156, et PGP « en
//! ligne » dans le texte). Cryptographie : `rpgp` (pur Rust, auditée) ; ce module ne fait que reconnaître la forme du
//! message, choisir les clés et rendre un verdict. Le contenu déchiffré n'est **jamais** écrit en base ni indexé : il
//! vit le temps de l'affichage.

use mail_parser::{MessageParser, MimeHeaders};
use pgp::composed::{
    CleartextSignedMessage, Deserializable, DetachedSignature, Message, MessageBuilder,
    SignedPublicKey, SignedSecretKey,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::sym::SymmetricKeyAlgorithm;
use pgp::types::{KeyDetails, Password, VerifyingKey};
use rand::thread_rng;
use serde::Serialize;

/// Taille maximale d'un élément PGP lu (clé, message, signature) : borne contre un message forgé.
pub const MAX_ARMORED: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SigState {
    /// Signature mathématiquement correcte, faite par une clé que tu as importée.
    Valid,
    /// La clé du signataire est connue mais la signature ne correspond pas au contenu : message altéré.
    Invalid,
    /// Aucune clé importée ne correspond au signataire : impossible de conclure.
    UnknownKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PgpSignature {
    pub state: SigState,
    /// Première identité (« Nom <adresse> ») de la clé qui a signé, si elle est connue.
    pub signer: Option<String>,
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DecryptError {
    /// Aucune clé privée importée ne correspond aux destinataires du message.
    NoKey,
    /// La clé existe mais sa phrase secrète manque ou est fausse.
    Passphrase,
    /// Message corrompu ou non pris en charge.
    Failed,
}

/// Forme d'un message PGP reconnue dans le message brut.
#[derive(Debug, PartialEq, Eq)]
pub enum Shape {
    /// `multipart/signed` : contenu signé (octets exacts de la première partie) et signature détachée.
    Signed {
        content: Vec<u8>,
        signature: Vec<u8>,
    },
    /// `multipart/encrypted` ou bloc `BEGIN PGP MESSAGE` dans le texte.
    /// `mime` : le contenu déchiffré est une entité MIME (PGP/MIME) ; sinon du texte brut (PGP en ligne).
    Encrypted { armored: Vec<u8>, mime: bool },
    /// Bloc `BEGIN PGP SIGNED MESSAGE` dans le texte.
    Cleartext { armored: String },
}

/// Mêmes octets avec des fins de ligne CRLF, comme les signe un client (RFC 3156 §5) : le transport peut les avoir normalisées.
fn crlf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 40);
    let mut prev = 0u8;
    for &b in data {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

/// Reconnaît la forme PGP d'un message brut. `None` : pas de PGP.
pub fn shape(raw: &[u8]) -> Option<Shape> {
    let msg = MessageParser::default().parse(raw)?;
    let root = msg.root_part();
    let ct = root.content_type()?;
    let subtype = ct.subtype().unwrap_or("").to_ascii_lowercase();
    let protocol = ct.attribute("protocol").unwrap_or("").to_ascii_lowercase();
    if ct.ctype().eq_ignore_ascii_case("multipart") {
        let kids = root.sub_parts()?;
        if subtype == "signed" && protocol == "application/pgp-signature" && kids.len() >= 2 {
            let first = msg.part(kids[0])?;
            let sig = msg.part(kids[1])?;
            let start = first.raw_header_offset() as usize;
            let end = first.raw_end_offset() as usize;
            let content = raw.get(start..end)?;
            return Some(Shape::Signed {
                content: crlf(content),
                signature: sig.contents().to_vec(),
            });
        }
        if subtype == "encrypted" && protocol == "application/pgp-encrypted" && kids.len() >= 2 {
            let data = msg.part(kids[1])?.contents();
            return Some(Shape::Encrypted {
                armored: data.to_vec(),
                mime: true,
            });
        }
        return None;
    }
    // PGP en ligne : le texte brut commence par un bloc armuré.
    let text = msg.body_text(0)?;
    let trimmed = text.trim_start();
    if trimmed.starts_with("-----BEGIN PGP MESSAGE-----") {
        let end = trimmed.find("-----END PGP MESSAGE-----")? + "-----END PGP MESSAGE-----".len();
        return Some(Shape::Encrypted {
            armored: trimmed.as_bytes()[..end].to_vec(),
            mime: false,
        });
    }
    if trimmed.starts_with("-----BEGIN PGP SIGNED MESSAGE-----") {
        let end =
            trimmed.find("-----END PGP SIGNATURE-----")? + "-----END PGP SIGNATURE-----".len();
        return Some(Shape::Cleartext {
            armored: trimmed[..end].to_string(),
        });
    }
    None
}

/// Clé publique importée : le certificat et ce qu'on en affiche.
pub struct PublicKey {
    pub cert: SignedPublicKey,
}

impl PublicKey {
    pub fn fingerprint(&self) -> String {
        self.cert.fingerprint().to_string().to_ascii_uppercase()
    }
    /// Identités, la première étant la principale.
    pub fn user_ids(&self) -> Vec<String> {
        self.cert
            .details
            .users
            .iter()
            .map(|u| String::from_utf8_lossy(u.id.id()).into_owned())
            .collect()
    }
    pub fn revoked(&self) -> bool {
        !self.cert.details.revocation_signatures.is_empty()
    }
}

/// Clé (publique ou privée) lue dans un fichier ou un presse-papiers.
pub enum Imported {
    Public(Box<PublicKey>),
    Secret(Box<SignedSecretKey>),
}

pub fn parse_key(data: &[u8]) -> Option<Imported> {
    if data.len() > MAX_ARMORED {
        return None;
    }
    if let Ok(text) = std::str::from_utf8(data) {
        if text.contains("-----BEGIN PGP") {
            if let Ok((k, _)) = SignedSecretKey::from_string(text) {
                return Some(Imported::Secret(Box::new(k)));
            }
            return SignedPublicKey::from_string(text)
                .ok()
                .map(|(cert, _)| Imported::Public(Box::new(PublicKey { cert })));
        }
    }
    // Export binaire (`gpg --export`).
    if let Ok(k) = SignedSecretKey::from_bytes(data) {
        return Some(Imported::Secret(Box::new(k)));
    }
    SignedPublicKey::from_bytes(data)
        .ok()
        .map(|cert| Imported::Public(Box::new(PublicKey { cert })))
}

/// Certificat public lu dans une valeur déjà connue pour être publique (annuaire).
pub fn parse_public(armored: &str) -> Option<PublicKey> {
    SignedPublicKey::from_string(armored)
        .ok()
        .map(|(cert, _)| PublicKey { cert })
}

/// Une clé publique par certificat ; la clé privée dont on dérive la publique pour l'annuaire.
pub fn public_of(secret: &SignedSecretKey) -> PublicKey {
    PublicKey {
        cert: SignedPublicKey::from(secret.clone()),
    }
}

/// Certificat public armuré (pour l'annuaire).
pub fn armor_public(k: &PublicKey) -> Option<String> {
    k.cert.to_armored_string(Default::default()).ok()
}

impl PgpSignature {
    /// Verdict rendu par la clé `k` (celle qui a signé).
    fn of(state: SigState, k: &PublicKey) -> Self {
        Self {
            state,
            signer: k.user_ids().into_iter().next(),
            fingerprint: Some(k.fingerprint()),
        }
    }
    /// Aucune clé connue ne correspond au signataire.
    fn unknown() -> Self {
        Self {
            state: SigState::UnknownKey,
            signer: None,
            fingerprint: None,
        }
    }
}

/// Une clé a-t-elle fait cette signature (primaire ou sous-clé) ? `Ok(true)` : valide ; `Ok(false)` : la clé est
/// bien l'émettrice mais la signature est fausse ; `Err` : cette clé n'est pas l'émettrice.
fn try_key(sig: &DetachedSignature, data: &[u8], key: &PublicKey) -> std::result::Result<bool, ()> {
    if sig.verify(&key.cert, data).is_ok() {
        return Ok(true);
    }
    for sub in &key.cert.public_subkeys {
        if sig.verify(sub, data).is_ok() {
            return Ok(true);
        }
    }
    // Émettrice ? On compare l'identifiant annoncé dans la signature à ceux de la clé.
    let issuers = sig.signature.issuer_key_id();
    let fprs = sig.signature.issuer_fingerprint();
    let ids = std::iter::once(key.cert.legacy_key_id())
        .chain(key.cert.public_subkeys.iter().map(|s| s.legacy_key_id()));
    let mut ids: Vec<_> = ids.collect();
    ids.dedup();
    let by_id = issuers.iter().any(|i| ids.contains(i));
    let by_fpr = fprs.iter().any(|f| {
        **f == key.cert.fingerprint()
            || key
                .cert
                .public_subkeys
                .iter()
                .any(|s| **f == s.fingerprint())
    });
    if by_id || by_fpr {
        Ok(false)
    } else {
        Err(())
    }
}

/// Verdict d'une signature détachée sur `data` (déjà en CRLF).
pub fn verify_detached(signature: &[u8], data: &[u8], keys: &[PublicKey]) -> Option<PgpSignature> {
    if signature.len() > MAX_ARMORED {
        return None;
    }
    let sig = parse_detached(signature)?;
    let mut issuer = None;
    for k in keys {
        match try_key(&sig, data, k) {
            Ok(true) => return Some(PgpSignature::of(SigState::Valid, k)),
            Ok(false) => issuer = Some(k),
            Err(()) => {}
        }
    }
    Some(issuer.map_or_else(PgpSignature::unknown, |k| {
        PgpSignature::of(SigState::Invalid, k)
    }))
}

fn parse_detached(bytes: &[u8]) -> Option<DetachedSignature> {
    let text = std::str::from_utf8(bytes).ok()?;
    if text.contains("-----BEGIN PGP SIGNATURE-----") {
        DetachedSignature::from_string(text).ok().map(|(s, _)| s)
    } else {
        DetachedSignature::from_bytes(bytes).ok()
    }
}

/// Signature d'un texte signé en clair (« clearsigned »).
pub fn verify_cleartext(armored: &str, keys: &[PublicKey]) -> Option<(String, PgpSignature)> {
    if armored.len() > MAX_ARMORED {
        return None;
    }
    let (msg, _) = CleartextSignedMessage::from_string(armored).ok()?;
    let text = msg.text().to_string();
    for k in keys {
        if msg.verify(&k.cert).is_ok()
            || k.cert.public_subkeys.iter().any(|s| msg.verify(s).is_ok())
        {
            return Some((text, PgpSignature::of(SigState::Valid, k)));
        }
    }
    Some((text, PgpSignature::unknown()))
}

/// Résultat d'un déchiffrement : contenu, signature interne éventuelle (message signé puis chiffré) et empreinte de la
/// clé qui a déchiffré (seule dont on retient la phrase secrète).
pub struct Decrypted {
    pub data: Vec<u8>,
    pub signature: Option<PgpSignature>,
    pub fingerprint: String,
}

/// Clé privée à essayer, avec la phrase secrète connue.
pub struct SecretKey {
    pub key: SignedSecretKey,
    pub fingerprint: String,
    pub passphrase: Option<String>,
}

fn locked(k: &SignedSecretKey) -> bool {
    k.primary_key.secret_params().is_encrypted()
        || k.secret_subkeys
            .iter()
            .any(|s| s.key.secret_params().is_encrypted())
}

fn parse_message(data: &[u8]) -> Option<Message<'_>> {
    match std::str::from_utf8(data) {
        Ok(t) if t.contains("-----BEGIN PGP MESSAGE-----") => {
            Message::from_string(t).ok().map(|(m, _)| m)
        }
        _ => Message::from_bytes(data).ok(),
    }
}

/// Déchiffre avec la première clé qui y parvient. Aucune ne réussit : `Passphrase` si l'une est verrouillée (une clé
/// verrouillée sans la bonne phrase est indiscernable d'une clé étrangère : demander la phrase est la seule suite
/// utile), sinon `NoKey`.
pub fn decrypt(
    data: &[u8],
    secrets: &[SecretKey],
    publics: &[PublicKey],
) -> std::result::Result<Decrypted, DecryptError> {
    if data.len() > MAX_ARMORED || parse_message(data).is_none() {
        return Err(DecryptError::Failed);
    }
    for s in secrets {
        // Le message est consommé par chaque essai : on le relit (analyse des seuls paquets d'en-tête, bon marché).
        let Some(msg) = parse_message(data) else {
            continue;
        };
        let password = s
            .passphrase
            .as_deref()
            .map_or_else(Password::empty, Password::from);
        let Ok(mut plain) = msg.decrypt(&password, &s.key) else {
            continue;
        };
        if plain.is_compressed() {
            plain = plain.decompress().map_err(|_| DecryptError::Failed)?;
        }
        let plain_data = plain.as_data_vec().map_err(|_| DecryptError::Failed)?;
        let signature = plain.is_signed().then(|| inner_signature(&plain, publics));
        return Ok(Decrypted {
            data: plain_data,
            signature,
            fingerprint: s.fingerprint.clone(),
        });
    }
    Err(if secrets.iter().any(|s| locked(&s.key)) {
        DecryptError::Passphrase
    } else {
        DecryptError::NoKey
    })
}

/// Signature portée dans le message chiffré (déjà lu en entier) : valide si l'une de nos clés publiques la confirme.
fn inner_signature(plain: &Message<'_>, publics: &[PublicKey]) -> PgpSignature {
    let refs: Vec<&dyn VerifyingKey> = publics
        .iter()
        .map(|k| &k.cert as &dyn VerifyingKey)
        .collect();
    let valid = plain.verify_nested(&refs).ok().and_then(|results| {
        publics
            .iter()
            .zip(results)
            .find(|(_, v)| matches!(v, pgp::composed::VerificationResult::Valid(_)))
            .map(|(k, _)| k)
    });
    valid.map_or_else(PgpSignature::unknown, |k| {
        PgpSignature::of(SigState::Valid, k)
    })
}

/// Échec d'une signature ou d'un chiffrement à l'envoi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealError {
    /// La clé de signature est verrouillée : phrase secrète absente ou fausse.
    Passphrase,
    /// Un destinataire n'a pas de sous-clé de chiffrement utilisable.
    NoEncryptionKey,
    Failed,
}

/// Sous-clé de signature de la clé, s'il y en a une (sinon la clé principale signe).
fn signing_subkey(key: &SignedSecretKey) -> Option<&pgp::composed::SignedSecretSubKey> {
    key.secret_subkeys
        .iter()
        .find(|s| s.signatures.iter().any(|g| g.key_flags().sign()))
}

/// Clé qui signe, pour un message OpenPGP (`MessageBuilder::sign`).
fn signing_key(key: &SignedSecretKey) -> &dyn pgp::types::SigningKey {
    match signing_subkey(key) {
        Some(s) => &s.key,
        None => &key.primary_key,
    }
}

/// Signature détachée (armurée) de `content`, avec la sous-clé de signature si la clé en a une, sinon la clé principale.
pub fn sign_detached(
    content: &[u8],
    key: &SignedSecretKey,
    passphrase: Option<&str>,
) -> std::result::Result<String, SealError> {
    let password = passphrase.map_or_else(Password::empty, Password::from);
    let hash = HashAlgorithm::Sha256;
    // `sign_binary_data` veut un type concret (pas `dyn`) : deux appels, une seule règle de choix de clé.
    let signed = match signing_subkey(key) {
        Some(s) => {
            DetachedSignature::sign_binary_data(thread_rng(), &s.key, &password, hash, content)
        }
        None => DetachedSignature::sign_binary_data(
            thread_rng(),
            &key.primary_key,
            &password,
            hash,
            content,
        ),
    };
    match signed {
        Ok(sig) => sig
            .to_armored_string(Default::default())
            .map_err(|_| SealError::Failed),
        Err(_) if locked(key) => Err(SealError::Passphrase),
        Err(_) => Err(SealError::Failed),
    }
}

/// Message chiffré (armuré) pour tous les `recipients`, avec la sous-clé de chiffrement de chacun. Avec `signer`, la
/// signature est portée **dans** le message chiffré (une seule enveloppe OpenPGP, comme GnuPG et Thunderbird : le
/// destinataire ne voit la signature qu'après avoir déchiffré).
pub fn encrypt_to(
    data: &[u8],
    recipients: &[&PublicKey],
    signer: Option<(&SignedSecretKey, Option<&str>)>,
) -> std::result::Result<String, SealError> {
    let mut builder = MessageBuilder::from_bytes("", data.to_vec())
        .seipd_v1(thread_rng(), SymmetricKeyAlgorithm::AES256);
    for r in recipients {
        let sub = r
            .cert
            .public_subkeys
            .iter()
            .find(|s| {
                s.signatures.iter().any(|g| {
                    let f = g.key_flags();
                    f.encrypt_comms() || f.encrypt_storage()
                })
            })
            .ok_or(SealError::NoEncryptionKey)?;
        builder
            .encrypt_to_key(thread_rng(), sub)
            .map_err(|_| SealError::Failed)?;
    }
    if let Some((key, passphrase)) = signer {
        let password = passphrase.map_or_else(Password::empty, Password::from);
        builder.sign(signing_key(key), password, HashAlgorithm::Sha256);
    }
    match builder.to_armored_string(thread_rng(), Default::default()) {
        Ok(armored) => Ok(armored),
        Err(_) if signer.is_some_and(|(k, _)| locked(k)) => Err(SealError::Passphrase),
        Err(_) => Err(SealError::Failed),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pgp::composed::{
        EncryptionCaps, KeyType, MessageBuilder, SecretKeyParamsBuilder, SubkeyParamsBuilder,
    };
    use pgp::crypto::ecc_curve::ECCCurve;
    use pgp::crypto::hash::HashAlgorithm;
    use pgp::crypto::sym::SymmetricKeyAlgorithm;
    use pgp::ser::Serialize;
    use rand::thread_rng;

    /// Clé Ed25519 + sous-clé de chiffrement, avec ou sans phrase secrète.
    pub(crate) fn keygen(uid: &str, passphrase: Option<&str>) -> SignedSecretKey {
        let mut enc = SubkeyParamsBuilder::default();
        enc.key_type(KeyType::ECDH(ECCCurve::Curve25519Legacy))
            .can_sign(false)
            .can_encrypt(EncryptionCaps::All)
            .can_authenticate(false)
            .passphrase(passphrase.map(str::to_string));
        let mut p = SecretKeyParamsBuilder::default();
        p.key_type(KeyType::Ed25519Legacy)
            .can_certify(true)
            .can_sign(true)
            .can_encrypt(EncryptionCaps::None)
            .primary_user_id(uid.into())
            .subkeys(vec![enc.build().unwrap()]);
        if let Some(pw) = passphrase {
            p.passphrase(Some(pw.to_string()));
        }
        p.build().unwrap().generate(thread_rng()).unwrap()
    }

    fn sk(k: &SignedSecretKey, passphrase: Option<&str>) -> SecretKey {
        SecretKey {
            key: k.clone(),
            fingerprint: String::new(),
            passphrase: passphrase.map(str::to_string),
        }
    }

    pub(crate) fn sign(key: &SignedSecretKey, data: &[u8]) -> Vec<u8> {
        DetachedSignature::sign_binary_data(
            thread_rng(),
            &key.primary_key,
            &Password::empty(),
            HashAlgorithm::Sha256,
            data,
        )
        .unwrap()
        .to_armored_bytes(Default::default())
        .unwrap()
    }

    pub(crate) fn encrypt(to: &SignedSecretKey, data: &[u8]) -> Vec<u8> {
        let pk = SignedPublicKey::from(to.clone());
        let mut b = MessageBuilder::from_bytes("", data.to_vec())
            .seipd_v1(thread_rng(), SymmetricKeyAlgorithm::AES256);
        b.encrypt_to_key(thread_rng(), &pk.public_subkeys[0])
            .unwrap();
        b.to_armored_string(thread_rng(), Default::default())
            .unwrap()
            .into_bytes()
    }

    pub(crate) fn mime_signed(content: &str, sig: &[u8]) -> Vec<u8> {
        format!(
            "From: a@x.fr\r\nTo: b@x.fr\r\nSubject: t\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; micalg=pgp-sha256; boundary=\"=_bnd\"\r\n\r\n\
             --=_bnd\r\n{content}\r\n--=_bnd\r\nContent-Type: application/pgp-signature\r\n\r\n{}\r\n--=_bnd--\r\n",
            String::from_utf8_lossy(sig)
        )
        .into_bytes()
    }

    pub(crate) const PART: &str = "Content-Type: text/plain; charset=utf-8\r\n\r\nbonjour\r\n";

    #[test]
    fn signature_pgp_mime_valide_puis_alteree() {
        let k = keygen("Alice <alice@x.fr>", None);
        let sig = sign(&k, PART.as_bytes());
        let raw = mime_signed(PART, &sig);
        let Some(Shape::Signed { content, signature }) = shape(&raw) else {
            panic!("forme signée attendue")
        };
        let keys = [public_of(&k)];
        let v = verify_detached(&signature, &content, &keys).unwrap();
        assert_eq!(v.state, SigState::Valid);
        assert_eq!(v.signer.as_deref(), Some("Alice <alice@x.fr>"));

        // Contenu modifié après signature.
        let tampered = mime_signed(&PART.replace("bonjour", "bonsoir"), &sig);
        let Some(Shape::Signed { content, signature }) = shape(&tampered) else {
            panic!()
        };
        assert_eq!(
            verify_detached(&signature, &content, &keys).unwrap().state,
            SigState::Invalid
        );

        // Clé inconnue.
        let other = public_of(&keygen("Eve <eve@x.fr>", None));
        let Some(Shape::Signed { content, signature }) = shape(&raw) else {
            panic!()
        };
        assert_eq!(
            verify_detached(&signature, &content, &[other])
                .unwrap()
                .state,
            SigState::UnknownKey
        );
    }

    #[test]
    fn signature_survit_a_la_normalisation_des_fins_de_ligne() {
        let k = keygen("Alice <alice@x.fr>", None);
        let sig = sign(&k, PART.as_bytes());
        // Le transport a remplacé CRLF par LF.
        let raw = String::from_utf8(mime_signed(PART, &sig))
            .unwrap()
            .replace("\r\n", "\n");
        let Some(Shape::Signed { content, signature }) = shape(raw.as_bytes()) else {
            panic!()
        };
        assert_eq!(
            verify_detached(&signature, &content, &[public_of(&k)])
                .unwrap()
                .state,
            SigState::Valid
        );
    }

    #[test]
    fn dechiffrement_avec_et_sans_phrase_secrete() {
        let k = keygen("Bob <bob@x.fr>", Some("sésame"));
        let enc = encrypt(&k, PART.as_bytes());
        let ok = decrypt(&enc, &[sk(&k, Some("sésame"))], &[]).unwrap();
        assert_eq!(ok.data, PART.as_bytes());
        assert_eq!(
            decrypt(&enc, &[sk(&k, None)], &[]).err(),
            Some(DecryptError::Passphrase)
        );
        assert_eq!(
            decrypt(&enc, &[sk(&k, Some("faux"))], &[]).err(),
            Some(DecryptError::Passphrase)
        );
        let autre = keygen("Eve <eve@x.fr>", None);
        assert_eq!(
            decrypt(&enc, &[sk(&autre, None)], &[]).err(),
            Some(DecryptError::NoKey)
        );
        assert_eq!(decrypt(&enc, &[], &[]).err(), Some(DecryptError::NoKey));
    }

    #[test]
    fn formes_reconnues_et_garbage_refuse() {
        let k = keygen("Bob <bob@x.fr>", None);
        let enc = String::from_utf8(encrypt(&k, b"x")).unwrap();
        let mime = format!(
            "Subject: s\r\nMIME-Version: 1.0\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"=_enc\"\r\n\r\n\
             --=_enc\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\r\n--=_enc\r\nContent-Type: application/octet-stream\r\n\r\n{enc}\r\n--=_enc--\r\n"
        );
        assert!(matches!(
            shape(mime.as_bytes()),
            Some(Shape::Encrypted { .. })
        ));
        let inline = format!("Subject: s\r\nContent-Type: text/plain\r\n\r\n{enc}\r\n");
        assert!(matches!(
            shape(inline.as_bytes()),
            Some(Shape::Encrypted { .. })
        ));
        assert_eq!(shape(b"Subject: s\r\n\r\nbonjour"), None);
        assert!(decrypt(
            b"-----BEGIN PGP MESSAGE-----\n\nnope\n-----END PGP MESSAGE-----",
            &[sk(&k, None)],
            &[]
        )
        .is_err());
    }

    #[test]
    fn import_de_cles() {
        let k = keygen("Alice <alice@x.fr>", None);
        let armored_pub = public_of(&k)
            .cert
            .to_armored_string(Default::default())
            .unwrap();
        let armored_sec = k.to_armored_string(Default::default()).unwrap();
        assert!(matches!(
            parse_key(armored_pub.as_bytes()),
            Some(Imported::Public(_))
        ));
        assert!(matches!(
            parse_key(armored_sec.as_bytes()),
            Some(Imported::Secret(_))
        ));
        assert!(parse_key(b"pas une cl\xc3\xa9").is_none());
        let Some(Imported::Public(p)) = parse_key(armored_pub.as_bytes()) else {
            panic!()
        };
        assert_eq!(p.user_ids(), vec!["Alice <alice@x.fr>"]);
        assert_eq!(p.fingerprint().len(), 40);
        assert!(!p.revoked());
        // Export binaire.
        let bin = public_of(&k).cert.to_bytes().unwrap();
        assert!(matches!(parse_key(&bin), Some(Imported::Public(_))));
    }
}
