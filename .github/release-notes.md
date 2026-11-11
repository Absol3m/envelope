## Build de test : non signé

Ces fichiers ne sont pas signés (pas de certificat Apple ni Microsoft pour l'instant) : le système les signale comme venant d'un développeur inconnu. C'est attendu.

**macOS (Apple Silicon)** : ouvrir le `.dmg`, glisser Envelope dans Applications, puis lancer dans un terminal :
`xattr -dr com.apple.quarantine /Applications/Envelope.app`
(ou clic droit → Ouvrir, puis Ouvrir dans la boîte de dialogue).

**Windows** : lancer l'installeur `.exe` ; si SmartScreen s'affiche : « Informations complémentaires » → « Exécuter quand même ».

**Linux** : `chmod +x Envelope_*.AppImage && ./Envelope_*.AppImage` (nécessite FUSE 2 : paquet `libfuse2`).

**Premiers pas** : ajouter un compte (mot de passe d'application pour Gmail, iCloud, Yahoo…). Le mot de passe est stocké dans le trousseau du système. Les données restent sur la machine (dossier de l'application).

**Retours** : décrire ce qui ne va pas avec le système, la version (ci-dessus) et, si possible, une capture.

**Vérifier un fichier** : `SHA256SUMS` donne les sommes de contrôle ; la provenance (construit par ce dépôt, depuis ce commit) se vérifie avec `gh attestation verify <fichier> --repo Absol3m/envelope`.
