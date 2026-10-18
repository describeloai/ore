# C0 · pool c0-medida (n2-standard-16 spot), 2026-10-18 — parcial

Leído del pod a las 12:22 antes de que lo desalojaran (12:31, disco efímero lleno). DuckDB 1.5.6, 48 GB, límite 1 200 s.

| datos | q1 | q2 | q3 | q4 | q5 | q6 | q7 | q8 | q9 | q10 | q11 | q12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| SF10 | 5,8 | 1,6 | 1,8 | 7,7 | 11,7 | 2,3 | 1,9 | 3,6 | 0,25 | límite | límite | límite |
| SF100 | 10,8 | 10,8 | 11,0 | 23,1 | — | — | — | — | — | — | — | — |

Banco a escala (preparar-banco.py en el pod): edificios 34,1 M en 31 s; AIS 7 días 115,7 M en 21 s.
Generar SpatialBench: SF10 6 min 38 s, SF100 17 min 21 s.
