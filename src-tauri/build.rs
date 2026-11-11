fn main() {
    // `sqlx::migrate!` ne détecte pas l'ajout d'un NOUVEAU fichier de migration : sans cette directive, le binaire
    // garde l'ancienne liste jusqu'à une recompilation forcée de la crate.
    println!("cargo:rerun-if-changed=migrations");
    tauri_build::build()
}
