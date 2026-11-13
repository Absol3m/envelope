//! Délai croissant entre deux essais : `base` doublé à chaque échec, plafonné à `max` (secondes).

use std::time::Duration;

/// Délai après `attempts` échecs déjà essuyés (0 : le premier essai).
pub fn delay_secs(base: i64, max: i64, attempts: i64) -> i64 {
    base.saturating_mul(1 << attempts.clamp(0, 20)).min(max)
}

/// Comme `delay_secs`, en `Duration` (pour `tokio::time::sleep` et `Instant`).
pub fn delay(base_secs: u64, max_secs: u64, attempts: u32) -> Duration {
    let secs = delay_secs(
        base_secs.min(i64::MAX as u64) as i64,
        max_secs.min(i64::MAX as u64) as i64,
        i64::from(attempts),
    );
    Duration::from_secs(secs as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_puis_plafonne_sans_deborder() {
        assert_eq!(
            (0..6).map(|n| delay_secs(30, 600, n)).collect::<Vec<_>>(),
            [30, 60, 120, 240, 480, 600]
        );
        assert_eq!(delay_secs(30, 3600, -3), 30);
        assert_eq!(delay_secs(30, 3600, i64::MAX), 3600);
        assert_eq!(delay_secs(i64::MAX, 3600, 20), 3600);
    }
}
