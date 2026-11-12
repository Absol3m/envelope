/// Copie du manifeste de Tauri (contrôles communs v6, dont dépendent les boîtes de dialogue natives).
const WINDOWS_MANIFEST: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/windows-app-manifest.xml");

fn main() {
    // `sqlx::migrate!` ne détecte pas l'ajout d'un NOUVEAU fichier de migration : sans cette directive, le binaire
    // garde l'ancienne liste jusqu'à une recompilation forcée de la crate.
    println!("cargo:rerun-if-changed=migrations");

    // Windows (MSVC) : Tauri ne lie son manifeste qu'à l'exécutable, et les binaires de test ne se chargent pas sans
    // lui (STATUS_ENTRYPOINT_NOT_FOUND) : on lie donc le même à tous les binaires, à la place du sien.
    // (Cible windows-gnu non couverte : elle garde le manifeste de Tauri, tests compris dans l'état d'avant.)
    let windows = if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rerun-if-changed={WINDOWS_MANIFEST}");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{WINDOWS_MANIFEST}");
        tauri_build::WindowsAttributes::new_without_app_manifest()
    } else {
        tauri_build::WindowsAttributes::new()
    };
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("préparation de l'application Tauri");
}
