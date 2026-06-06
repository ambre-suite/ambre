# AMBRE — ROADMAP TECHNIQUE

> **Rôle de ce fichier.** Il porte l'**ordre transversal** du projet : la séquence des
> *milestones* inter-crates et leurs dépendances. Il ne réénumère aucune tâche.
> Le **détail et l'état d'avancement** (`[x]/[ ]`) vivent dans le §7 de chaque spec
> `1-Foundations/`, référencés ici par ID préfixé (`ARCH P1`, `SYNC P2`, `SRV P0`,
> `CANVAS P0`, `CHROME P1`, `UI P1`).
>
> **Stratégie :** livraison verticale de Trame (valider la stack complète sur un produit),
> puis expansion horizontale. Pas d'estimation en semaines : l'ordre relatif des milestones
> et leurs dépendances sont la seule contrainte engageante.

---

## M0 — Socle technique

*Objectif : la stack 3-threads tourne, le contenu se rend, le chrome est monté, la crypto est prouvée.*

- [x] Threading shared-memory + crates de base + IPC typé : `ARCH P0`, `ARCH P1` (partiel).
- [x] Pipeline de rendu GPU (`FrameHost`/`DocView`, fonts, grille Trame 60+ FPS) : `CANVAS P0`.
- [x] Chrome Leptos (app shell, commandes, saisie/IME, tokens, a11y) : `CHROME P1`→`P3`, `UI P0`→`P2`.
- [x] Primitives E2EE standalone (`ambre-crypto`, pipeline compress/seal, dérivation `MasterSeed`) : `SYNC P0`.

## M1 — Trame vertical chiffré & synchronisé

*Objectif : un Trame 100% fonctionnel, chiffré de bout en bout et synchronisé via le Blind Relay.*
*C'est le milestone qui valide la stack entière.*

- [x] **Édition Trame typée** (`TrameOutcome`, dispatcher Logic lossless) : `ARCH P2`.
- [ ] **Sync réseau réel** : boucle non-bloquante + WebSocket multiplexé, `Unlock`/`SetupAccount`,
  `OpenDocument`/`CloseDocument`, CRUD documents : `ARCH P1` (items restants), `SYNC P1` (câblé) → `SYNC P2`.
- [x] **Blind Relay** : PostgreSQL/Redis/S3, WS multiplexé, ACL, Lua clock+seq, presigned URLs : `SRV P0`.
- [x] **Finitions canvas** : tokens générés, tests headless : `CANVAS P0` (items restants).

> Dépendances : `SRV P0` précède le câblage réseau de `SYNC P2` (besoin d'un relais cible) ;
> `SYNC P0` précède `SYNC P1`/`P2` (crypto avant intégration).

## M2 — Identité, partage & durcissement

*Objectif : multi-utilisateurs sécurisé, partage/révocation, socle prêt pour Enterprise.*

- [ ] Partage & révocation (rotation Document Key, médias, anti-replay) : `SYNC P3`.
- [ ] Durcissement serveur (rate limiting, CSRF, organisations + Shamir, compaction) : `SRV P2`.
- [ ] Durabilité offline (persistance Loro IndexedDB, flush + `RequestMissing` au reconnect) : `ARCH P4`.
- [ ] Moteur formules Trame complet (DAG, recalcul incrémental, détection de cycles) : `ARCH P4`.

## M3 — Expansion applicative

*Objectif : déployer les autres briques sur le socle validé.*

- [ ] `ambre-prose` (LoroText), `ambre-toile` (R-Tree), `ambre-murmure` (channels) : `ARCH P2`/`ARCH P5`.
- [ ] Sync channels/meetings (segments gérés par Sync, signaling WebRTC) : `SYNC P4`.
- [ ] Serveur channels + meetings (segments, unread, signaling) : `SRV P1`.
- [ ] Canvas paths/transform + export PDF Krilla : `CANVAS P1`.

## M4 — Polish & Rendez-Vous avancé

*Objectif : qualité de lancement, Beta publique.*

- [ ] Profiling frame budget, thème dynamique, i18n/RTL, benchmarks de non-régression : `CANVAS P2`.
- [ ] Rendez-Vous : chiffrement SFrame (E2EE vidéo) : `SYNC P4` / Rendez-Vous.
- [ ] Audit de sécurité ZK final, audit warnings wasm-only : `SYNC P4`, `ARCH P4`.
