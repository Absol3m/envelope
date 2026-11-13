//! Heure courante en secondes Unix (UTC) : file d'envoi, relances, jetons OAuth, copies dans « Envoyés ».

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
