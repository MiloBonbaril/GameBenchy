# Baselines GameBenchy

3 seeds × 3 runs, 9 parties par agent. Métrique principale : vague atteinte.

| agent | vague moy. | min–max | actions | illégales | rounds forcés | déplacements | or remboursé | tokens/vague | JSON illisibles |
|---|---|---|---|---|---|---|---|---|---|
| random | **5.7** | 4–8 | 48.0 | 27.0 | 0.2 | 2.7 | 151 | — | 0 |
| greedy | **24.3** | 24–25 | 39.7 | 0.0 | 0.0 | 0.0 | 0 | — | 0 |
| gemma | **6.1** | 5–8 | 38.6 | 5.3 | 0.1 | 4.3 | 267 | 17068 | 15 |
| qwen | **11.6** | 7–20 | 24.7 | 3.4 | 0.0 | 0.0 | 0 | 3391 | 0 |

## Profil d'erreurs (total par code)

| agent | PATH_BLOCKED | INSUFFICIENT_GOLD | CELL_OCCUPIED | NO_MOVES_LEFT | INVALID_CELL | WRONG_PHASE | UNKNOWN_BUILDING |
|---|---|---|---|---|---|---|---|
| random | 0 | 215 | 4 | 17 | 7 | 0 | 0 |
| greedy | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| gemma | 1 | 9 | 1 | 37 | 0 | 0 | 0 |
| qwen | 0 | 31 | 0 | 0 | 0 | 0 | 0 |
