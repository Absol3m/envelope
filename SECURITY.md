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

## Données au repos
Les messages, leur index de recherche et les pièces jointes sont stockés **en clair** dans le dossier de données de l'application : Envelope ne chiffre pas ses fichiers (comme Thunderbird, Apple Mail ou Outlook). La protection repose sur :
- le **chiffrement de disque du système** (FileVault, BitLocker, LUKS) : une pastille dans la barre latérale indique s'il est actif ;
- les **droits des fichiers** : sous Unix, le dossier de données est en `0700` et les fichiers en `0600` (utilisateur seul, ni groupe ni autres ; `root` passe toujours outre les droits) ; sous Windows, les listes de contrôle d'accès héritées du profil. Les exportations (mbox, eml) suivent le `umask` du dossier choisi, et un serveur MCP lancé par un autre compte ne pourra plus lire la base ;
- le **verrouillage de l'application** (mot de passe, Touch ID, Windows Hello), qui n'est **pas** un chiffrement : un programme qui lit directement les fichiers n'est pas arrêté, ni une sauvegarde du profil ;
- les secrets (mots de passe, jetons OAuth) restent dans le trousseau du système, jamais dans ces fichiers.
Penser à chiffrer les sauvegardes du dossier de données. Le chiffrement de la base elle-même a été étudié puis écarté pour l'instant (voir l'issue #9).

## Chaîne d'approvisionnement
Dépendances surveillées par Dependabot et par `cargo deny` (RustSec, licences, sources) / `pnpm audit` dans la CI ; actions GitHub de publication épinglées par empreinte ; fichiers de version accompagnés de `SHA256SUMS` et d'une attestation de provenance (`gh attestation verify <fichier> --repo Absol3m/envelope`).
