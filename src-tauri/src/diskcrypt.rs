//! Le disque qui contient les données d'Envelope est-il chiffré par le système ? (FileVault, BitLocker, LUKS)
//!
//! On interroge l'outil du système lui-même, sans droits d'administrateur ; au moindre doute (outil absent,
//! sortie inattendue, délai dépassé) le résultat est `Unknown` et l'interface n'affiche rien : mieux vaut
//! se taire qu'afficher une fausse alerte ou une fausse assurance. L'analyse des sorties est faite par des
//! fonctions pures, testées.
//!
//! Aucune crate ni API native utilisable ici : l'état de ces trois systèmes ne s'obtient que par leurs outils
//! publics (`fdesetup`, `lsblk`/`findmnt`) ou, sous Windows, par une propriété du Shell ; les API de bas niveau
//! (WMI, FSCTL) exigent les droits d'administrateur ou du code `unsafe`.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use ts_rs::TS;

use serde::Serialize;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum DiskEncryption {
    On,
    Off,
    Unknown,
}

const TIMEOUT: Duration = Duration::from_secs(5);

/// Lance une commande système et rend sa sortie standard ; `None` si elle échoue ou dépasse le délai.
async fn run(program: &str, args: &[&str], env: &[(&str, &str)]) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW : pas de fenêtre de console qui clignote
    let out = tokio::time::timeout(TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// macOS : `fdesetup status` → « FileVault is On. » / « FileVault is Off. ».
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // testé sur tous les systèmes, utilisé sur le sien seulement
pub fn parse_filevault(out: &str) -> DiskEncryption {
    let first = out.lines().next().unwrap_or("").trim();
    if first.starts_with("FileVault is On") {
        DiskEncryption::On
    } else if first.starts_with("FileVault is Off") {
        DiskEncryption::Off
    } else {
        DiskEncryption::Unknown
    }
}

/// Windows : propriété `System.Volume.BitLockerProtection` du lecteur système (1 = protégé, 0 = non chiffré).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn parse_bitlocker(out: &str) -> DiskEncryption {
    match out.trim() {
        "1" => DiskEncryption::On,
        "0" => DiskEncryption::Off,
        _ => DiskEncryption::Unknown,
    }
}

/// Linux : types de périphériques de `lsblk -s` (le volume et ses parents) ; « crypt » = dm-crypt/LUKS.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_lsblk_types(out: &str) -> DiskEncryption {
    let types: Vec<&str> = out.split_whitespace().collect();
    if types.is_empty() {
        DiskEncryption::Unknown
    } else if types.contains(&"crypt") {
        DiskEncryption::On
    } else {
        DiskEncryption::Off
    }
}

/// `findmnt` peut rendre « /dev/mapper/x[/@] » (sous-volume btrfs) : on ne garde que le périphérique.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn device_of(findmnt: &str) -> Option<&str> {
    let dev = findmnt.trim().split('[').next()?.trim();
    dev.starts_with('/').then_some(dev)
}

/// macOS : FileVault chiffre le volume de démarrage (volume « Data »). Si le dossier de données est ailleurs
/// (disque externe, autre volume), l'état de FileVault ne dit rien de lui : `Unknown`.
#[cfg(target_os = "macos")]
pub async fn status(data_dir: &Path) -> DiskEncryption {
    use std::os::unix::fs::MetadataExt;
    let device = |p: &str| std::fs::metadata(p).map(|m| m.dev()).ok();
    let same_volume = device(&data_dir.to_string_lossy()) == device("/System/Volumes/Data");
    if !same_volume {
        return DiskEncryption::Unknown;
    }
    run("fdesetup", &["status"], &[])
        .await
        .map_or(DiskEncryption::Unknown, |o| parse_filevault(&o))
}

/// Windows : lecteur du dossier de données (jamais interpolé dans le script : passé par variable d'environnement).
/// Hypothèse **non vérifiée sur une vraie machine** : valeurs 1 (protégé) et 0 (non chiffré) ; le reste → `Unknown`.
#[cfg(windows)]
pub async fn status(data_dir: &Path) -> DiskEncryption {
    use std::path::{Component, Prefix};
    let drive = match data_dir.components().next() {
        Some(Component::Prefix(p)) => match p.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                format!("{}:\\", char::from(letter))
            }
            _ => return DiskEncryption::Unknown,
        },
        _ => return DiskEncryption::Unknown,
    };
    let script = "(New-Object -ComObject Shell.Application).NameSpace($env:ENVELOPE_DRIVE).Self.ExtendedProperty('System.Volume.BitLockerProtection')";
    run(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
        &[("ENVELOPE_DRIVE", &drive)],
    )
    .await
    .map_or(DiskEncryption::Unknown, |o| parse_bitlocker(&o))
}

/// Linux : périphérique qui porte le dossier de données (`findmnt -T`), puis ses parents (`lsblk -s`) : « crypt » = LUKS.
#[cfg(target_os = "linux")]
pub async fn status(data_dir: &Path) -> DiskEncryption {
    let Some(dir) = data_dir.to_str() else {
        return DiskEncryption::Unknown;
    };
    let Some(src) = run("findmnt", &["-n", "-o", "SOURCE", "-T", dir], &[]).await else {
        return DiskEncryption::Unknown;
    };
    let Some(dev) = device_of(&src) else {
        return DiskEncryption::Unknown;
    };
    run("lsblk", &["-s", "-n", "-r", "-o", "TYPE", dev], &[])
        .await
        .map_or(DiskEncryption::Unknown, |o| parse_lsblk_types(&o))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub async fn status(_data_dir: &Path) -> DiskEncryption {
    DiskEncryption::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filevault() {
        assert_eq!(parse_filevault("FileVault is On.\n"), DiskEncryption::On);
        assert_eq!(parse_filevault("FileVault is Off.\n"), DiskEncryption::Off);
        assert_eq!(
            parse_filevault("Encryption in progress: Percent completed = 40\n"),
            DiskEncryption::Unknown
        );
        assert_eq!(parse_filevault(""), DiskEncryption::Unknown);
    }

    #[test]
    fn bitlocker() {
        assert_eq!(parse_bitlocker("1\r\n"), DiskEncryption::On);
        assert_eq!(parse_bitlocker("0\r\n"), DiskEncryption::Off);
        assert_eq!(parse_bitlocker(""), DiskEncryption::Unknown);
        assert_eq!(parse_bitlocker("Erreur"), DiskEncryption::Unknown);
    }

    #[test]
    fn lsblk() {
        assert_eq!(
            parse_lsblk_types("lvm\ncrypt\npart\ndisk\n"),
            DiskEncryption::On
        );
        assert_eq!(parse_lsblk_types("part\ndisk\n"), DiskEncryption::Off);
        assert_eq!(parse_lsblk_types("\n"), DiskEncryption::Unknown);
    }

    #[test]
    fn peripherique_du_montage() {
        assert_eq!(
            device_of("/dev/mapper/luks-1234\n"),
            Some("/dev/mapper/luks-1234")
        );
        assert_eq!(
            device_of("/dev/mapper/root[/@home]\n"),
            Some("/dev/mapper/root")
        );
        assert_eq!(device_of("tmpfs\n"), None);
        assert_eq!(device_of(""), None);
    }

    #[tokio::test]
    async fn commande_absente_donne_none() {
        assert_eq!(run("envelope-commande-inexistante", &[], &[]).await, None);
    }
}
