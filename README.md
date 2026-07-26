# GameBenchy

**Un benchmark d'agents LLM déguisé en jeu de tower defense.**

GameBenchy est un environnement d'évaluation reproductible pour agents LLM, construit autour d'un jeu de défense par vagues avec maze-building. Le round de préparation est un problème de planification pur (état complet visible, optimisation spatiale + économique), ce qui isole le raisonnement stratégique. La génération procédurale et les mécaniques originales éliminent la contamination par les corpus d'entraînement (contrairement aux benchmarks basés sur NetHack).

> **Statut : v0.4 — premier run comparatif.** Moteur déterministe, TUI, serveur HTTP, runner de benchmark et les deux modes d'observation sont implémentés ; baselines vs 2 LLM locaux en §11, comparaison minimal/detailed en §12. Le reste de ce document fige les décisions de design ; §9, §10, §11 et §12 notent les écarts entre le design et le code.

## Démarrer

```sh
cargo run -p tui -- 42          # jouer une partie (seed 42)
cargo run -p server             # API sur 127.0.0.1:3000, base gamebenchy.db
cargo test                      # moteur + API + agents + rendu
cargo run -p runner --release   # campagne baselines → results/campaign.{csv,md}
cargo run -p runner --release -- balance   # harnais d'équilibrage (table mono-tourelle)
```

Campagne avec un LLM local (serveur OpenAI-compatible : llama.cpp, Ollama, vLLM) :

```sh
cargo run -p runner --release -- --agent greedy --agent llm:gemma@127.0.0.1:8080 --max-waves 25
cargo run -p runner --release -- --agent llm:gemma@127.0.0.1:8080 --mode detailed  # lore noyé
```

Au TUI : `hjkl`/flèches déplacent le curseur, `1`–`5` construisent, `s` vend, `m` déplace, `Entrée` lance la vague, `r` rejoue la même seed, `q` quitte.

---

## 1. Principes de benchmark (non négociables)

1. **Reproductibilité totale.** Même seed + même séquence d'actions = même partie, bit pour bit. RNG seedé unique, zéro dépendance à l'horloge. Tout score est attaché à une version du moteur.
2. **Gradient de difficulté continu.** La difficulté croît avec les vagues → métrique continue naturelle, jamais saturée.
3. **Anti-contamination.** Mécaniques originales, génération procédurale, thème propre.
4. **Métriques multiples et interprétables.** Chaque métrique raconte une capacité distincte (voir §6).
5. **Interface agent triviale.** `act(observation) -> action`, rien de plus.
6. **Dev seeds / eval seeds séparées.** Seeds publiques pour développer, seeds cachées pour classer.
7. **Coût borné.** Limite d'actions par round, limite de vagues, limite de tokens.
8. **Baselines non-LLM.** Agent random + agent scripté greedy, sinon les scores LLM sont ininterprétables.

---

## 2. Le jeu

### Concept

Tower defense / base building par rounds sur plateau ≤ 10x10. Les ennemis entrent par une case d'**entrée** et cherchent la **sortie** (min. une case d'écart). Boucle :

1. **Round de préparation** — l'agent construit, vend, déplace, avec son or.
2. **Round de résolution** — la vague attaque, le moteur simule de façon déterministe, rapport retourné.
3. Retour en 1. **Survival infini scoré**, cible ~100 vagues max par partie.

### Pathfinding & maze-building (mécanique centrale)

- Les ennemis terrestres suivent le **plus court chemin** entrée → sortie (BFS/A*, coût uniforme), recalculé à chaque résolution.
- Poser des bâtiments **modifie le chemin** : le labyrinthe est la défense.
- **Blocage interdit** : tout placement rendant la sortie inaccessible est rejeté avec `PATH_BLOCKED`. Les tentatives sont comptées (métrique de compréhension spatiale).
- **Tie-breaking déterministe et documenté** entre chemins de même longueur (ordre de priorité directionnel fixe).
- Les **volants ignorent le labyrinthe** : ligne droite entrée → sortie.

### Condition de défaite

Compteur de **vies**. Chaque ennemi atteignant la sortie retire des vies (infanterie : 1, blindé : 3, volant : 1 — à équilibrer). 0 vie = fin de partie, **mort permanente stricte**. Score = vague atteinte.

### Bâtiments (5)

| Bâtiment | Rôle | Trade-off |
|---|---|---|
| Sniper | Anti-infanterie mono-cible, longue portée | Seule tourelle anti-aérienne |
| Lance-flammes | AoE courte portée | Fort sur groupes, aveugle au ciel |
| Anti-blindage | Anti-char mono-cible | Faible contre infanterie |
| Mortier | AoE longue portée, cadence faible | Fort sur vagues denses, nul sur rushs |
| Bâtiment éco | +revenu par round | Ne défend pas : investissement vs survie |

### Ennemis (3 en v1)

| Ennemi | Counter | Particularité |
|---|---|---|
| Infanterie | Sniper, lance-flammes | Nombreuse |
| Blindé | Anti-blindage | -3 vies si fuite |
| Volant | Sniper uniquement | Ignore le labyrinthe |

Grille de counters : chaque ennemi a un counter clair, chaque tourelle une identité. 2 ennemis supplémentaires possibles après équilibrage (v1.x).

### Économie

- Or de départ + **revenu par round** (augmenté par les bâtiments éco).
- **Vente** : remboursement 50 % du coût.
- **Déplacement** : gratuit mais **rationné à 1/round** (à équilibrer, éventuellement 2). Deux leviers, deux prix, un arbitrage — et un signal riche de gaspillage/adaptation.

### Information

- **Vague N+1** : composition exacte visible (pas l'ordre d'apparition).
- **Vague N+2** : annonce textuelle (`incoming_intel`).
- **Rapport post-vague** : kills, fuites, vies perdues, **dégâts par bâtiment** (feedback granulaire indispensable à l'adaptation en cours de partie).

### Deux modes d'observation

Même schéma JSON, seul le contenu de `incoming_intel` (et du lore) change :

- **`minimal`** : lore sec ("armor incoming").
- **`detailed`** : flavor text abondant, l'information tactique est **enfouie dans le texte** → test needle-in-haystack intégré au benchmark.

### Direction artistique

Dieselpunk guerres mondiales fantaisistes (réf. : *Iron Harvest*, *Saga of Tanya the Evil*). Rendu **ASCII** en v1. Le lore détaillé fait partie de l'éval (mode `detailed`).

---

## 3. Architecture

```
gamebenchy/  (workspace Rust)
├── engine/    # moteur pur, déterministe, zéro I/O
├── server/    # API HTTP (axum), validation, persistance SQLite
├── tui/       # client humain (ratatui) : jouer, équilibrer, débugger
└── runner/    # benchmark : adaptateurs d'agents, campagnes, métriques
```

- **Moteur** : Rust, déterministe, seedé. Aucune dépendance au temps réel.
- **API** : HTTP simple (pas de WebSocket en v1).
- **Persistance** : SQLite — seed + **log d'actions complet** (l'état n'est pas stocké, il se rejoue) → toute partie est rejouable, débuggable, auditable par un tiers.
- **Observation** : JSON uniquement en v1.

---

## 4. Contrat agent / moteur

**Schéma figé en v0.2** — implémenté par `server/`, testé par `server/src/main.rs`. Toute évolution ultérieure est une rupture de contrat explicite.

Base : `http://127.0.0.1:3000`. Trois endpoints. La partie est désignée par `?game_id=gN` en query string (`POST /game` le retourne).

### `POST /game`

```json
{"seed": 42, "mode": "minimal"}
```
→ `{"ok": true, "state": {...}}`, avec `state.game_id`. `mode` est optionnel (défaut `minimal`).

### `GET /state?game_id=g1`

```json
{
  "ok": true,
  "state": {
    "game_id": "g1",
    "seed": 42,
    "mode": "minimal",
    "wave": 7,
    "phase": "preparation",
    "lives": 12,
    "gold": 85,
    "income_per_wave": 38,
    "board": {
      "width": 10, "height": 10,
      "entry": {"x": 0, "y": 4},
      "exit": {"x": 9, "y": 5},
      "buildings": [
        {"id": "b1", "type": "sniper", "x": 3, "y": 2, "damage_dealt": 540}
      ]
    },
    "current_path": [{"x": 0, "y": 4}, {"x": 1, "y": 4}],
    "next_wave": {"composition": {"infantry": 8, "armor": 2, "flyer": 0}},
    "incoming_intel": "N+2 : des moteurs lourds grondent au loin.",
    "moves_remaining": 1,
    "actions_remaining": 20,
    "shop": [
      {"type": "sniper", "cost": 50},
      {"type": "flamethrower", "cost": 40},
      {"type": "anti_armor", "cost": 70},
      {"type": "mortar", "cost": 90},
      {"type": "eco", "cost": 60}
    ],
    "last_wave_report": {
      "wave": 6,
      "kills": {"infantry": 7, "armor": 2, "flyer": 0},
      "leaked": {"infantry": 1, "armor": 0, "flyer": 0},
      "lives_lost": 1,
      "damage_by_building": {"b1": 340}
    }
  }
}
```

`phase` ∈ `preparation` | `game_over`. `last_wave_report` est `null` avant la première vague. L'ordre des clés JSON n'est pas significatif.

Notes de design :

- **`current_path` est fourni** en v1 (isole la stratégie de la géométrie). Le masquer plus tard = une variante d'éval "simulation spatiale mentale" gratuite.
- **`incoming_intel`** est le canal unique du flavor text (modes minimal/detailed) et annonce la vague **N+2**.
- **`last_wave_report`** rend l'adaptation mesurable.
- **`actions_remaining`** expose la limite dure : sans elle l'agent ne peut pas savoir qu'il va se faire clôturer son tour.

### `POST /action?game_id=g1`

```json
{"action": "build", "building_type": "sniper", "x": 3, "y": 5}
{"action": "sell", "building_id": "b2"}
{"action": "move", "building_id": "b1", "x": 4, "y": 6}
{"action": "end_turn"}
```

Plusieurs actions par round de préparation, clôturées par `end_turn`.

**Limite dure : 20 actions tentées par round** (erreurs comprises), au-delà le moteur force `end_turn`. Borne le coût en tokens ; le nombre de rounds forcés est une métrique.

### Réponses & erreurs catégorisées

```json
{"ok": true, "state": {...}}
```
```json
{"ok": false, "error_code": "PATH_BLOCKED", "message": "Ce placement rendrait la sortie inaccessible."}
```

Codes de règle (HTTP 400) : `PATH_BLOCKED` · `INSUFFICIENT_GOLD` · `CELL_OCCUPIED` · `NO_MOVES_LEFT` · `INVALID_CELL` · `WRONG_PHASE` · `UNKNOWN_BUILDING`.
Codes de protocole : `UNKNOWN_ACTION` (400, JSON non conforme au contrat) · `UNKNOWN_GAME` (404).

Le runner compte les erreurs **par catégorie** → profil d'incompréhension par modèle (spatial vs économique vs protocole).

### Persistance & rejeu

SQLite, deux tables : `games(id, seed, mode)` et `actions(game_id, n, payload)`. **L'état n'est jamais stocké** : il est rejoué depuis le seed + le log à chaque requête, le moteur étant déterministe. Conséquences :

- toute partie est auditable action par action (`SELECT payload FROM actions WHERE game_id = 1 ORDER BY n`) ;
- le serveur peut redémarrer en cours de partie sans rien perdre ;
- les actions **en erreur sont journalisées aussi** — elles consomment un crédit d'action et alimentent les métriques, les omettre ferait diverger le rejeu.

---

## 5. Protocole d'évaluation

- **3 seeds × 3 runs = 9 parties par agent**, moyenne. Implémenté par `runner/`.
- Interface agent : `act(observation) -> action` (`runner/src/agents.rs`). L'observation est **exactement** le JSON du §4 (le runner appelle le même code que le serveur) ; le seul canal d'action est celui du §4, avec les mêmes limites et les mêmes erreurs.
- Agents v1 : random, greedy scripté, LLM locaux (tout serveur OpenAI-compatible : llama.cpp, Ollama, vLLM), puis modèles API si concluant (budget ~100 €).
- Comparaison **mode minimal vs mode detailed** sur les mêmes seeds = résultat de benchmark en soi.
- Seeds d'éval distinctes des seeds de dev : **101, 102, 103** en éval, 1/2/3/42/1337 en dev.

## 6. Métriques

| Métrique | Ce qu'elle mesure |
|---|---|
| **Vague atteinte** (principale, leaderboard) | Performance globale |
| Actions illégales, par code d'erreur | Compréhension des règles (spatiale / éco / protocole) |
| Tentatives `PATH_BLOCKED` | Raisonnement spatial |
| Or gaspillé (ventes à perte, éco inutilisée) | Gestion de ressources |
| Déplacements utilisés | Adaptation |
| Rounds forcés (limite d'actions) | Convergence décisionnelle |
| Tokens par vague | Efficience |
| Écart minimal vs detailed | Robustesse au bruit (needle-in-haystack) |

---

## 7. Roadmap

Budget : ~7 h/semaine. Deadline : **premier run comparatif publiable avant octobre 2026**.

### v0.1 — Moteur jouable (semaines 1–4) — **fait**
Structs, génération de vagues seedée, pathfinding + no-block + tie-breaking, boucle prépa/résolution, simulation déterministe, vies, score. TUI pour jouer soi-même.
**Jalon : perdre une partie au TUI et la trouver intéressante.** L'équilibrage se fait ici, en jouant — pas en théorisant. Ne pas avancer tant que le jeu est trivial.

État : `cargo run -p tui -- 42` joue une partie complète jusqu'à la mort. Premier passage d'équilibrage fait au harnais scripté (`cargo run -p runner --release -- balance`, l'agent greedy de v0.3) : agent greedy mort vague ~24 en moyenne (5 seeds), première fuite vague ~5 (arrivée des blindés), aucune tourelle dominante (meilleure stratégie mono-tourelle : vague 10). Validé à la main au TUI : le jeu est exigeant et plaisant à jouer.

### v0.2 — API + persistance (semaines 5–6) — **fait**
Serveur axum, 3 endpoints, validation + erreurs catégorisées, SQLite (seed + log d'actions).
**Jalon : schéma JSON figé et documenté** (contrat stable avant le runner).

État : `cargo run -p server` sert les trois endpoints ; le schéma ci-dessus (§4) est le contrat figé, testé bout en bout (`cargo test -p server`). La persistance ne stocke que le seed et le log d'actions : l'état est rejoué à chaque requête, donc une partie survit au redémarrage du serveur et reste auditable action par action. Écarts de contrat par rapport au cadrage initial : §10.

### v0.3 — Runner + baselines (semaines 7–8) — **fait**
Interface agent, adaptateur LLM local, agents random + greedy, campagne 3×3, sortie CSV/markdown.
**Jalon : premier tableau baselines vs 2 LLM locaux.**

État : `cargo run -p runner --release` joue la campagne 3 seeds × 3 runs et écrit `results/campaign.csv` + `results/campaign.md` (§11). L'adaptateur LLM parle à n'importe quel serveur OpenAI-compatible (`--agent llm:nom@host:port`, suffixe `+nothink` pour couper le raisonnement), donc ajouter un modèle = un flag ; les campagnes se fusionnent d'une passe à l'autre, ce qui permet de comparer des modèles qui ne tiennent pas ensemble en mémoire. **Jalon atteint** : random, greedy et deux LLM locaux (Gemma-4-E2B et Qwen3.6-27B) dans le même tableau.

### v0.4 — Modes d'observation + campagne (semaines 9–10) — **fait**
Mode `detailed` (écriture du lore), run comparatif minimal vs detailed, README de résultats.
**Jalon final : le premier run comparatif publiable.**

État : `--mode detailed` sert le même schéma JSON avec un `incoming_intel` bavard (`engine::incoming_intel`), seedé donc rejouable. **Jalon atteint** : les quatre agents ont joué les mêmes 9 parties dans les deux modes, tableau et lecture en §12. Deux enseignements pour la suite — les agents scriptés donnent un écart exactement nul (le mode ne touche qu'au texte), et le needle-in-haystack ne discriminera qu'au-dessus du niveau de jeu actuel. Écart de calendrier : livré avant la deadline d'octobre 2026, la marge prévue n'a pas servi.

### v2 (hors scope v1)
Interface web humaine (comparaison humain/LLM), mode stream (agent verbalisant + injection d'événements par les viewers), catégorie séparée "LLM + harness/outils", char ralentisseur, ennemis 4–5, format d'observation ASCII/texte, masquage de `current_path`.

---

## 8. Décisions figées (récapitulatif)

| Question | Décision |
|---|---|
| Nature du projet | Benchmark d'abord, jeu ensuite |
| Genre | TD / base building avec maze-pathfinding |
| Plateau | ≤ 10x10, entrée/sortie fixes |
| Défaite | Vies, mort permanente stricte |
| Scoring | Survival infini, vague atteinte |
| Bâtiments | Sniper, lance-flammes, anti-blindage, mortier, éco |
| Ennemis v1 | Infanterie, blindé, volant (ignore le maze) |
| Blocage du chemin | Interdit, rejeté, compté |
| Vente / déplacement | 50 % / gratuit limité à 1 par round |
| Info vagues | N+1 : composition exacte ; N+2 : intel textuelle |
| Observation | JSON, modes minimal & detailed |
| Stack | Rust workspace : engine, server (axum), tui (ratatui), runner |
| Persistance | SQLite, seed + log d'actions (replay complet) |
| Éval | 3 seeds × 3 runs, moyenne ; baselines random + greedy |
| Limite | 20 actions/round, ~100 vagues max |
| Budget | LLM locaux d'abord, ~100 € si concluant |

---

## 9. Ce que v0.1 a tranché (le code fait foi : `engine/src/lib.rs`)

Décisions prises en implémentant, non couvertes ou laissées ouvertes par le cadrage :

| Point | Choix v0.1 | Pourquoi |
|---|---|---|
| Gradient de difficulté | PV ennemis **+8 %/vague, composé** | Le revenu (donc la DPS achetable) croît linéairement : la partie finit toujours, sans palier ni saturation |
| **Blindage** (nouvelle mécanique) | Le blindé retranche 6 dégâts de chaque coup non spécialisé | Sans ça le sniper généraliste répondait à tout : plus de counter, plus d'arbitrage |
| Arrivée des types | Blindés vague 5, volants vague 8 | Laisse le temps d'apprendre une menace à la fois |
| Économie | 180 or de départ, +30/vague, éco +8 | Permet de réagir à un counter à 70 or sans rendre le spam de tourelles gratuit |
| Vies | 20 ; infanterie 1, blindé 3, volant 1 | Une vague ratée coûte cher sans finir la partie |
| Tie-breaking du chemin | BFS, ordre d'exploration **Droite, Bas, Haut, Gauche** | Déterministe et documenté (README §2) |
| Ordre d'un tick | spawn → tir → morts → déplacement | Figé : c'est ce qui rend la simulation rejouable |
| Ciblage | L'ennemi le plus avancé à portée, id croissant en cas d'égalité | Déterministe, et lisible pour un agent |
| Ordre d'apparition | Fisher-Yates seedé (composition connue, ordre non) | Conforme au contrat §2 |
| RNG | xorshift64* écrit dans le dépôt | La reproductibilité ne peut pas dépendre d'une version de crate |
| Résolution au TUI | Rapport immédiat, pas d'animation | v0.1 sert à équilibrer, pas à regarder ; animation = v0.2+ si utile |

Tout ça reste à ré-équilibrer en jouant : `cargo run -p runner --release -- balance` mesure, le TUI tranche.

---

## 10. Ce que v0.2 a tranché (le code fait foi : `server/src/main.rs`)

Le schéma §4 est figé ; voici où il s'écarte de l'esquisse initiale et pourquoi.

| Point | Choix v0.2 | Pourquoi |
|---|---|---|
| Désignation de la partie | `?game_id=gN` en query string, id court séquentiel | L'esquisse ne disait pas où passait le `game_id` ; un id opaque suffit, un UUID coûterait une dépendance pour rien |
| `hp` des bâtiments | Remplacé par `damage_dealt` | Les bâtiments ne sont pas attaquables en v1 : le champ aurait été une constante ; le cumul de dégâts, lui, sert à l'agent |
| `kills` / `leaked` | Les deux sont des objets `{infantry, armor, flyer}` | L'esquisse mélangeait objet et liste ; un seul type de composition partout |
| `actions_remaining` | Ajouté à l'état | La limite dure est une règle du jeu : la cacher pénaliserait l'agent sur du protocole, pas sur de la stratégie |
| `mode` | Accepté, persisté, renvoyé ; le lore `detailed` est écrit depuis v0.4 | Le contrat a été figé avant le texte, pour que l'écriture du lore ne puisse pas déplacer le schéma |
| Erreurs de protocole | `UNKNOWN_ACTION` (400) et `UNKNOWN_GAME` (404) en plus des 7 codes de règle | Un JSON malformé n'est pas une erreur de jeu : le runner doit pouvoir séparer les deux profils |
| Stockage de l'état | Aucun : seed + log rejoués à chaque requête | Une seule source de vérité, aucun cache à invalider, rejeu et audit gratuits ; un cache mémoire viendra si la latence se voit |
| Journalisation des erreurs | Les actions refusées sont écrites au log comme les autres | Elles consomment un crédit d'action et incrémentent les métriques : les omettre ferait diverger le rejeu |
---

## 11. Premiers résultats (v0.3) — `results/campaign.md`

Campagne du 25 juillet 2026, seeds d'éval 101/102/103 × 3 runs, plafond 25 vagues, observation en mode `minimal`.
LLM locaux, sur une carte de 16 Go :

- **gemma** = Gemma-4-E2B-it (Q4_K_XL, entièrement sur GPU), raisonnement actif.
- **qwen** = Qwen3.6-27B (IQ4_XS, ~15 Go : débordement partiel sur CPU), **raisonnement coupé** — avec raisonnement le modèle tourne à ~5 tokens/s, soit plus de 2 h par partie. C'est un écart de configuration assumé : les deux lignes ne mesurent pas le même régime de calcul.

3 seeds × 3 runs, 9 parties par agent. Métrique principale : vague atteinte.

| agent | vague moy. | min–max | actions | illégales | rounds forcés | déplacements | or remboursé | tokens/vague | JSON illisibles |
|---|---|---|---|---|---|---|---|---|---|
| random | **5.7** | 4–8 | 48.0 | 27.0 | 0.2 | 2.7 | 151 | — | 0 |
| greedy | **24.3** | 24–25 | 39.7 | 0.0 | 0.0 | 0.0 | 0 | — | 0 |
| gemma | **6.1** | 5–8 | 38.6 | 5.3 | 0.1 | 4.3 | 267 | 17068 | 15 |
| qwen | **11.6** | 7–20 | 24.7 | 3.4 | 0.0 | 0.0 | 0 | 3391 | 0 |

### Profil d'erreurs (total par code)

| agent | PATH_BLOCKED | INSUFFICIENT_GOLD | CELL_OCCUPIED | NO_MOVES_LEFT | INVALID_CELL | WRONG_PHASE | UNKNOWN_BUILDING |
|---|---|---|---|---|---|---|---|
| random | 0 | 215 | 4 | 17 | 7 | 0 | 0 |
| greedy | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| gemma | 1 | 9 | 1 | 37 | 0 | 0 | 0 |
| qwen | 0 | 31 | 0 | 0 | 0 | 0 | 0 |

Ce que ça dit, et pourquoi c'est encourageant pour le benchmark :

- **Le benchmark discrimine, et dans le bon ordre.** greedy 24.3 > qwen 11.6 > gemma 6.1 > random 5.7. Le petit modèle joue à peine mieux que le hasard, le gros fait deux fois mieux mais reste à moitié du plancher scripté : il reste toute la place pour classer des modèles plus forts, sans effet plafond.
- **La variance est le signal, pas le bruit.** qwen va de la vague 7 à la vague 20 selon le run : le jeu récompense une bonne suite de décisions, il ne se gagne pas au tirage.
- **Le profil d'erreurs sépare les capacités.** Le random se cogne à l'économie (215 `INSUFFICIENT_GOLD`) ; gemma comprend les prix mais pas le rationnement (37 `NO_MOVES_LEFT` : il redemande des déplacements déjà consommés) ; qwen ne fait plus que des erreurs de budget (31), jamais de protocole. Aucun des trois ne déclenche `PATH_BLOCKED` : personne ne construit encore assez pour buter sur le raisonnement spatial, qui reste donc une capacité **non mesurée** à ce niveau de jeu.
- **Le gaspillage se mesure.** 267 or remboursés par partie chez gemma (il achète puis revend), 0 chez qwen et le greedy.
- **Coût.** 17 k tokens par vague survécue pour gemma (raisonnement compris), 3,4 k pour qwen sans raisonnement ; ~7 min par partie contre ~2 min. Une campagne 3×3 se compte en heures pour le premier, en dizaines de minutes pour le second.
- **JSON illisibles : 15 chez gemma, 0 chez qwen.** La cause mesurée est la troncature du raisonnement : sous 1 400 tokens de réponse, gemma n'atteint jamais son objet JSON (9 échecs sur 7 actions à 700 tokens, 1 sur 14 à 1 400). Ce n'est pas une incapacité à produire du JSON, c'est un budget de sortie trop court — à retenir pour tout modèle à raisonnement.

Les deux modèles ne tiennent pas ensemble sur la carte : la campagne se fait en deux passes, le runner **fusionne** le CSV existant (un agent rejoué remplace ses anciennes lignes, supprimer `results/campaign.csv` remet le tableau à zéro).

---

## 12. Robustesse au bruit (v0.4) — `results/campaign.md`

Le mode `detailed` noie les mêmes faits dans un rapport de renseignement bavard : sept à huit phrases de vie de bataillon (pluie, courrier, groupes électrogènes) parmi lesquelles se cache la seule information tactique — l'annonce de la vague N+2. Le remplissage est **vrai et tactiquement vide** : une phrase qui mentirait sur les ennemis testerait la crédulité, pas la lecture. Tout le reste de l'observation est identique au bit près, et le texte reste une fonction pure de `(seed, vague)` : le rejeu d'une partie reproduit le même rapport.

Mêmes seeds, mêmes runs, même plafond de 25 vagues que le §11.

| agent | minimal | detailed | écart | tokens/vague |
|---|---|---|---|---|
| random | 5.7 | 5.7 | **+0.0** | — |
| greedy | 24.3 | 24.3 | **+0.0** | — |
| gemma | 6.1 | 5.4 | **−0.7** | 17 068 → 18 927 (+11 %) |
| qwen | 11.6 | 13.9 | **+2.3** | 3 391 → 3 625 (+7 %) |

Lecture :

- **Le contrôle est propre.** Les agents scriptés ne lisent pas `incoming_intel` : leur écart est exactement nul, ce qui prouve que le mode ne touche qu'au texte et jamais aux règles.
- **Aucun effet mesurable sur les LLM à ce niveau.** −0,7 d'un côté, +2,3 de l'autre, sur 9 parties dont l'étendue va de 7 à 21 vagues : les deux écarts tiennent dans la variance run à run. Le foin ne les fait pas tomber — mais ils meurent d'économie et de rationnement d'actions **bien avant** que l'annonce N+2 soit ce qui limite leur score. Le test needle-in-haystack est donc en place et instrumenté, mais **il ne discrimine pas encore** : il ne mordra que sur des modèles assez forts pour que la préparation de la vague N+2 décide de la partie.
- **Le bruit se paie en tokens, pas en vagues** : +7 à +11 % par vague survécue. C'est le seul effet robuste des deux lignes.

### Un piège de validité, corrigé

La première campagne `detailed` de qwen a rendu 3,7 vagues de moyenne avec 66 réponses illisibles et zéro token compté : le serveur, encore en chargement, répondait 200 sur `/v1/models` et **503** sur `/v1/chat/completions`, et le runner comptait chaque 503 comme une mauvaise réponse du modèle. Un modèle injoignable obtenait ainsi un score, plus bas que le hasard. Le runner distingue désormais le statut HTTP du contenu : une erreur de transport **casse la campagne** au lieu de publier un chiffre inventé (`runner/src/llm.rs`). Les chiffres ci-dessus sont ceux du rejeu contre un serveur réellement chargé.
