# GameBenchy

**Un benchmark d'agents LLM déguisé en jeu de tower defense.**

GameBenchy est un environnement d'évaluation reproductible pour agents LLM, construit autour d'un jeu de défense par vagues avec maze-building. Le round de préparation est un problème de planification pur (état complet visible, optimisation spatiale + économique), ce qui isole le raisonnement stratégique. La génération procédurale et les mécaniques originales éliminent la contamination par les corpus d'entraînement (contrairement aux benchmarks basés sur NetHack).

> **Statut : v0 — cadrage.** Ce document fige les décisions de design. Rien n'est codé.

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
- **Persistance** : SQLite — seed + état courant + **log d'actions complet** → toute partie est rejouable, débuggable, auditable par un tiers.
- **Observation** : JSON uniquement en v1.

---

## 4. Contrat agent / moteur

### `POST /game`

```json
{"seed": 42, "mode": "minimal"}
```
→ `game_id` + état initial.

### `GET /state`

```json
{
  "game_id": "uuid",
  "seed": 42,
  "wave": 7,
  "phase": "preparation",
  "lives": 12,
  "gold": 85,
  "income_per_wave": 25,
  "board": {
    "width": 10, "height": 10,
    "entry": {"x": 0, "y": 4},
    "exit": {"x": 9, "y": 5},
    "buildings": [
      {"id": "b1", "type": "sniper", "x": 3, "y": 4, "hp": 100}
    ]
  },
  "current_path": [{"x": 0, "y": 4}, {"x": 1, "y": 4}],
  "next_wave": {"composition": {"infantry": 8, "armor": 2, "flyer": 0}},
  "incoming_intel": "Des moteurs lourds grondent au loin...",
  "moves_remaining": 1,
  "shop": [
    {"type": "sniper", "cost": 50},
    {"type": "flamethrower", "cost": 40},
    {"type": "anti_armor", "cost": 70},
    {"type": "mortar", "cost": 90},
    {"type": "eco", "cost": 60}
  ],
  "last_wave_report": {
    "leaked": [{"type": "infantry", "count": 1}],
    "lives_lost": 1,
    "kills": {"infantry": 7, "armor": 2},
    "damage_by_building": {"b1": 340}
  }
}
```

Notes de design :

- **`current_path` est fourni** en v1 (isole la stratégie de la géométrie). Le masquer plus tard = une variante d'éval "simulation spatiale mentale" gratuite.
- **`incoming_intel`** est le canal unique du flavor text (modes minimal/detailed).
- **`last_wave_report`** rend l'adaptation mesurable.

### `POST /action`

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

Codes : `PATH_BLOCKED` · `INSUFFICIENT_GOLD` · `CELL_OCCUPIED` · `NO_MOVES_LEFT` · `INVALID_CELL` · `WRONG_PHASE` · `UNKNOWN_BUILDING`.

Le runner compte les erreurs **par catégorie** → profil d'incompréhension par modèle (spatial vs économique vs protocole).

---

## 5. Protocole d'évaluation

- **3 seeds × 3 runs = 9 parties par agent**, moyenne.
- Interface agent : `act(observation) -> action`.
- Agents v1 : random, greedy scripté, LLM locaux (Ollama), puis modèles API si concluant (budget ~100 €).
- Comparaison **mode minimal vs mode detailed** sur les mêmes seeds = résultat de benchmark en soi.
- Seeds d'éval distinctes des seeds de dev.

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

### v0.1 — Moteur jouable (semaines 1–4)
Structs, génération de vagues seedée, pathfinding + no-block + tie-breaking, boucle prépa/résolution, simulation déterministe, vies, score. TUI pour jouer soi-même.
**Jalon : perdre une partie au TUI et la trouver intéressante.** L'équilibrage se fait ici, en jouant — pas en théorisant. Ne pas avancer tant que le jeu est trivial.

### v0.2 — API + persistance (semaines 5–6)
Serveur axum, 3 endpoints, validation + erreurs catégorisées, SQLite (seed + log d'actions).
**Jalon : schéma JSON figé et documenté** (contrat stable avant le runner).

### v0.3 — Runner + baselines (semaines 7–8)
Interface agent, adaptateur Ollama, agents random + greedy, campagne 3×3, sortie CSV/markdown.
**Jalon : premier tableau baselines vs 2 LLM locaux.**

### v0.4 — Modes d'observation + campagne (semaines 9–10)
Mode `detailed` (écriture du lore), run comparatif minimal vs detailed, README de résultats.
**Jalon final : le premier run comparatif publiable.**

*Marge : si le moteur déborde (l'équilibrage déborde toujours), v0.4 glisse après la deadline sans compromettre le jalon v0.3.*

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