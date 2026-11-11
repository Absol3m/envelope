# Politique de sécurité

## Signaler une vulnérabilité
**Ne la publiez pas dans une issue.** Utilisez le signalement privé de GitHub : onglet *Security* du dépôt → *Report a vulnerability* (s'il n'apparaît pas, ouvrez une issue qui demande un contact privé, sans détail technique). Indiquez la version d'Envelope, le système, les étapes pour reproduire et l'impact que vous voyez. Pas de vrais identifiants ni de vrais e-mails dans le rapport.

Envelope est maintenu par une seule personne : réponse et correctif au mieux, sans délai garanti. Le rapporteur est cité dans les notes de version s'il le souhaite.

## Versions prises en charge
Envelope est en pré-version : seule la dernière version publiée reçoit des correctifs.

## Ce qui est considéré comme sensible
- Le contenu des e-mails est une entrée **non fiable** (HTML, en-têtes, pièces jointes, noms de dossiers IMAP) : toute exécution de code, fuite de données ou contournement du blocage des contenus distants qui en découle est une vulnérabilité.
- Secrets (mots de passe, jetons OAuth2) : trousseau du système uniquement, jamais en base ni dans les journaux.
- Certificats TLS : toujours vérifiés par le système, sans exception globale.
- Serveur MCP : lecture seule sur la base ; tout contenu d'e-mail qu'il renvoie est signalé comme non fiable.

## Chaîne d'approvisionnement
Dépendances surveillées par Dependabot et par `cargo deny` (RustSec, licences, sources) / `pnpm audit` dans la CI ; actions GitHub de publication épinglées par empreinte ; fichiers de version accompagnés de `SHA256SUMS` et d'une attestation de provenance (`gh attestation verify <fichier> --repo Absol3m/envelope`).
