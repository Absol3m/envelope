//! Apparence du système hôte, pour que l'application en reprenne les couleurs.
//!
//! - [`host`] : faits fixes, connus sans attente (palette de base, matière de fenêtre), injectés
//!   dans la page avant le premier affichage.
//! - [`read`] : réglages que l'utilisateur peut changer en cours de route (accent, clair/sombre
//!   du bureau, schéma KDE), relus quand l'application revient au premier plan.
//!
//! Sources : registre sous Windows, `AppleAccentColor` sous macOS, portail XDG
//! `org.freedesktop.appearance` (GNOME 47+, KDE Plasma 6…) et `kdeglobals` sous Linux.
//! Tout est facultatif : une valeur absente laisse l'interface sur sa palette par défaut.

use serde::Serialize;

/// Faits fixes du système hôte.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Host {
    /// Palette de base : `"windows"`, `"macos"`, `"kde"` ou `"gnome"` (autres bureaux Linux).
    pub palette: &'static str,
    pub material: Option<Material>,
}

/// Matière translucide native sous le contenu de la fenêtre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Material {
    /// Windows 11.
    Mica,
    /// macOS (barre latérale).
    Vibrancy,
}

/// Réglages d'apparence courants.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Appearance {
    /// Préférence sombre du bureau, que la vue web ne voit pas toujours (Linux).
    pub dark: Option<bool>,
    /// Couleur d'accent (`#rrggbb`) pour le thème clair, puis pour le thème sombre.
    pub accent_light: Option<String>,
    pub accent_dark: Option<String>,
    /// Schéma de couleurs de l'utilisateur (KDE).
    pub palette: Option<Palette>,
}

/// Couleurs principales d'un schéma de bureau (`#rrggbb`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    /// Fond des fenêtres (barre latérale, chrome).
    pub window: String,
    /// Fond des vues de contenu (listes, lecteur).
    pub view: String,
    pub fg: String,
    pub fg_muted: String,
}

#[cfg_attr(target_os = "macos", allow(dead_code))] // macOS : couleurs fixes, déjà en hexadécimal
fn hex(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

pub fn host() -> Host {
    #[cfg(target_os = "windows")]
    return Host {
        palette: "windows",
        material: windows::supports_mica().then_some(Material::Mica),
    };
    #[cfg(target_os = "macos")]
    return Host {
        palette: "macos",
        material: Some(Material::Vibrancy),
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    Host {
        palette: linux_palette(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default()),
        material: None,
    }
}

pub async fn read() -> Appearance {
    #[cfg(target_os = "windows")]
    return windows::read();
    #[cfg(target_os = "macos")]
    return macos::read();
    #[cfg(target_os = "linux")]
    return linux::read().await;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Appearance::default()
}

// ───────────── Windows ─────────────

/// `AccentPalette` : 8 couleurs RGBA, des plus claires aux plus foncées
/// (Light3, Light2, Light1, Base, Dark1, Dark2, Dark3, —). Fluent utilise Dark1 en thème clair
/// et Light2 en thème sombre.
#[cfg(any(target_os = "windows", test))]
fn windows_accents(palette: &[u8]) -> Option<(String, String)> {
    let at = |i: usize| palette.get(i * 4..i * 4 + 3).map(|c| hex(c[0], c[1], c[2]));
    Some((at(4)?, at(1)?))
}

/// `AccentColor` du DWM : 0xAABBGGRR.
#[cfg(any(target_os = "windows", test))]
fn windows_dwm_color(v: u32) -> String {
    hex(v as u8, (v >> 8) as u8, (v >> 16) as u8)
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    /// Mica : Windows 11 (build 22000 et plus).
    pub fn supports_mica() -> bool {
        RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
            .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
            .ok()
            .and_then(|b| b.parse::<u32>().ok())
            .is_some_and(|build| build >= 22000)
    }

    /// Le mode clair/sombre n'est pas lu : WebView2 le reflète déjà (`prefers-color-scheme`).
    pub fn read() -> Appearance {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let accents = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent")
            .and_then(|k| k.get_raw_value("AccentPalette"))
            .ok()
            .and_then(|v| windows_accents(&v.bytes))
            .or_else(|| {
                let c = hkcu
                    .open_subkey(r"Software\Microsoft\Windows\DWM")
                    .and_then(|k| k.get_value::<u32, _>("AccentColor"))
                    .ok()?;
                Some((windows_dwm_color(c), windows_dwm_color(c)))
            });
        let (accent_light, accent_dark) = accents.unzip();
        Appearance {
            accent_light,
            accent_dark,
            ..Default::default()
        }
    }
}

// ───────────── macOS ─────────────

/// Couleur d'accent choisie dans Réglages Système (`AppleAccentColor`), en clair et en sombre.
/// Absente = « multicolore », c'est-à-dire le bleu par défaut.
#[cfg(any(target_os = "macos", test))]
fn macos_accents(index: Option<i32>) -> (&'static str, &'static str) {
    match index {
        Some(-1) => ("#8c8c8c", "#989898"), // graphite
        Some(0) => ("#ff3b30", "#ff453a"),  // rouge
        Some(1) => ("#ff9500", "#ff9f0a"),  // orange
        Some(2) => ("#ffcc00", "#ffd60a"),  // jaune
        Some(3) => ("#28cd41", "#32d74b"),  // vert
        Some(5) => ("#af52de", "#bf5af2"),  // violet
        Some(6) => ("#ff2d55", "#ff375f"),  // rose
        _ => ("#007aff", "#0a84ff"),        // bleu
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    pub fn read() -> Appearance {
        let index = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleAccentColor"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok());
        let (light, dark) = macos_accents(index);
        Appearance {
            accent_light: Some(light.into()),
            accent_dark: Some(dark.into()),
            ..Default::default()
        }
    }
}

// ───────────── Linux ─────────────

/// Palette d'après `XDG_CURRENT_DESKTOP` (p. ex. `ubuntu:GNOME`, `KDE`) ; libadwaita par défaut.
#[cfg(any(not(any(target_os = "windows", target_os = "macos")), test))]
fn linux_palette(xdg: &str) -> &'static str {
    if xdg.split(':').any(|d| d.eq_ignore_ascii_case("KDE")) {
        "kde"
    } else {
        "gnome"
    }
}

/// Couleur du portail (composantes 0–1) ; hors bornes = non définie.
#[cfg(any(target_os = "linux", test))]
fn portal_color(r: f64, g: f64, b: f64) -> Option<String> {
    let c = |v: f64| (0.0..=1.0).contains(&v).then(|| (v * 255.0).round() as u8);
    Some(hex(c(r)?, c(g)?, c(b)?))
}

/// Schéma de couleurs KDE (`~/.config/kdeglobals`, couleurs `r,g,b`) et couleur de sélection.
#[cfg(any(target_os = "linux", test))]
fn kde_palette(ini: &str) -> (Option<Palette>, Option<String>) {
    let mut section = "";
    let mut entries = std::collections::HashMap::new();
    for line in ini.lines().map(str::trim) {
        if let Some(s) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = s;
        } else if let Some((k, v)) = line.split_once('=') {
            entries.insert((section, k.trim()), v.trim());
        }
    }
    let color = |section: &str, key: &str| {
        let mut it = entries
            .get(&(section, key))?
            .split(',')
            .map(|n| n.trim().parse::<u8>());
        Some(hex(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?))
    };
    let palette = match (
        color("Colors:Window", "BackgroundNormal"),
        color("Colors:View", "BackgroundNormal"),
        color("Colors:Window", "ForegroundNormal"),
        color("Colors:Window", "ForegroundInactive"),
    ) {
        (Some(window), Some(view), Some(fg), Some(fg_muted)) => Some(Palette {
            window,
            view,
            fg,
            fg_muted,
        }),
        _ => None,
    };
    (palette, color("Colors:Selection", "BackgroundNormal"))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use ashpd::desktop::settings::{ColorScheme, Settings};
    use std::time::Duration;

    /// Le portail peut manquer (bureau minimal) : on n'attend pas plus d'une demi-seconde.
    async fn portal() -> (Option<bool>, Option<String>) {
        let query = async {
            let settings = Settings::new().await.ok()?;
            let dark = match settings.color_scheme().await {
                Ok(ColorScheme::PreferDark) => Some(true),
                Ok(ColorScheme::PreferLight) => Some(false),
                _ => None,
            };
            let accent = settings
                .accent_color()
                .await
                .ok()
                .and_then(|c| portal_color(c.red(), c.green(), c.blue()));
            Some((dark, accent))
        };
        tokio::time::timeout(Duration::from_millis(500), query)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    pub async fn read() -> Appearance {
        let (dark, mut accent) = portal().await;
        let mut palette = None;
        if host().palette == "kde" {
            if let Some(ini) =
                dirs::config_dir().and_then(|d| std::fs::read_to_string(d.join("kdeglobals")).ok())
            {
                let (p, selection) = kde_palette(&ini);
                accent = accent.or(selection);
                palette = p;
            }
        }
        Appearance {
            dark,
            accent_light: accent.clone(),
            accent_dark: accent,
            palette,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_palette_and_dwm() {
        // Palette par défaut de Windows 11 (bleu), 8 × RGBA.
        let p: Vec<u8> = [
            [0x99, 0xeb, 0xff],
            [0x4c, 0xc2, 0xff],
            [0x00, 0x91, 0xf8],
            [0x00, 0x78, 0xd4],
            [0x00, 0x67, 0xc0],
            [0x00, 0x3e, 0x92],
            [0x00, 0x1a, 0x68],
            [0xf7, 0x63, 0x0c],
        ]
        .iter()
        .flat_map(|c| [c[0], c[1], c[2], 0xff])
        .collect();
        assert_eq!(
            windows_accents(&p),
            Some(("#0067c0".into(), "#4cc2ff".into()))
        );
        assert_eq!(windows_accents(&p[..8]), None);
        assert_eq!(windows_dwm_color(0xffd47800), "#0078d4");
    }

    #[test]
    fn macos_accent_table() {
        assert_eq!(macos_accents(None), ("#007aff", "#0a84ff"));
        assert_eq!(macos_accents(Some(5)).0, "#af52de");
        assert_eq!(macos_accents(Some(-1)).0, "#8c8c8c");
    }

    #[test]
    fn linux_helpers() {
        assert_eq!(linux_palette("ubuntu:GNOME"), "gnome");
        assert_eq!(linux_palette("KDE"), "kde");
        assert_eq!(linux_palette("XFCE"), "gnome");
        assert_eq!(portal_color(0.2078, 0.5176, 0.8941), Some("#3584e4".into()));
        assert_eq!(portal_color(-1.0, 0.0, 0.0), None);
    }

    #[test]
    fn kde_scheme() {
        let ini = "[General]\nColorScheme=BreezeDark\n\n[Colors:View]\nBackgroundNormal=20,22,24\n\n\
                   [Colors:Window]\nBackgroundNormal=32,35,38\nForegroundNormal=252,252,252\nForegroundInactive=161,169,177\n\n\
                   [Colors:Selection]\nBackgroundNormal=61,174,233\n";
        let (p, sel) = kde_palette(ini);
        let p = p.expect("palette");
        assert_eq!(p.window, "#202326");
        assert_eq!(p.view, "#141618");
        assert_eq!(p.fg_muted, "#a1a9b1");
        assert_eq!(sel.as_deref(), Some("#3daee9"));
        assert_eq!(kde_palette("[General]\n").0, None);
    }
}
