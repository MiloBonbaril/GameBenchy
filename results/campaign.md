# Baselines GameBenchy

3 seeds × 3 runs, 9 parties par agent. Métrique principale : vague atteinte.

| agent | vague moy. | min–max | actions | illégales | rounds forcés | déplacements | or remboursé | tokens/vague | JSON illisibles |
|---|---|---|---|---|---|---|---|---|---|
| random | **5.7** | 4–8 | 48.0 | 27.0 | 0.2 | 2.7 | 151 | — | 0 |
| greedy | **24.3** | 24–25 | 39.7 | 0.0 | 0.0 | 0.0 | 0 | — | 0 |
| gemma | **6.1** | 5–8 | 38.6 | 5.3 | 0.1 | 4.3 | 267 | 17068 | 15 |
| qwen | **11.6** | 7–20 | 24.7 | 3.4 | 0.0 | 0.0 | 0 | 3391 | 0 |
| random+detailed | **5.7** | 4–8 | 48.0 | 27.0 | 0.2 | 2.7 | 151 | — | 0 |
| greedy+detailed | **24.3** | 24–25 | 39.7 | 0.0 | 0.0 | 0.0 | 0 | — | 0 |
| gemma+detailed | **5.4** | 4–8 | 36.8 | 5.1 | 0.0 | 3.9 | 276 | 18927 | 11 |
| qwen+detailed | **13.9** | 7–21 | 28.3 | 2.9 | 0.0 | 0.0 | 0 | 3625 | 0 |

## Robustesse au bruit : minimal vs detailed

| agent | minimal | detailed | écart | tokens/vague min → det |
|---|---|---|---|---|
| random | 5.7 | 5.7 | **+0.0** | — → — |
| greedy | 24.3 | 24.3 | **+0.0** | — → — |
| gemma | 6.1 | 5.4 | **-0.7** | 17068 → 18927 |
| qwen | 11.6 | 13.9 | **+2.3** | 3391 → 3625 |

## Profil d'erreurs (total par code)

| agent | PATH_BLOCKED | INSUFFICIENT_GOLD | CELL_OCCUPIED | NO_MOVES_LEFT | INVALID_CELL | WRONG_PHASE | UNKNOWN_BUILDING |
|---|---|---|---|---|---|---|---|
| random | 0 | 215 | 4 | 17 | 7 | 0 | 0 |
| greedy | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| gemma | 1 | 9 | 1 | 37 | 0 | 0 | 0 |
| qwen | 0 | 31 | 0 | 0 | 0 | 0 | 0 |
| random+detailed | 0 | 215 | 4 | 17 | 7 | 0 | 0 |
| greedy+detailed | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| gemma+detailed | 0 | 11 | 0 | 35 | 0 | 0 | 0 |
| qwen+detailed | 0 | 25 | 1 | 0 | 0 | 0 | 0 |
