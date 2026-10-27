# Elastic Sessions

> **Cerrado (2026-10-25) para el paradigma de Query.** La imagen limpia —qué se promete, cómo funciona,
> las decisiones, lo medido, dónde está cada pieza, cómo operarlo y la deuda— está en
> [`products/0073-elastic-sessions.md`](products/0073-elastic-sessions.md). Este documento fue su hoja
> de ruta mientras se construía; queda sólo para saber de dónde viene cada pieza.

| paso | qué | commit |
|---|---|---|
| S0 · previos | Kueue no ve lo crecido; bajar funciona; la lista cerrada; el precio | — (medido) |
| S1 · el session pool | `n2-highmem-8` spot con SSD, mínimo 0, `singleProcessOOMKill`; las peligrosas mueren solas | en vivo 2026-10-24 |
| S2a · `ore-serve` redimensiona | `POST /puestos/{id}/scale`, `elastica.rs`, `ore-redimensionar` | `main` |
| S2b · la cuenta y executions | presupuesto de 40 GiB, `quota`; `growth`, `memory_final_mb`, `growth_refused` | `main` |
| S3a · el kernel en su proceso | `KernelAparte`; un OOM da `OutOfMemoryError` y la sesión sigue | `main` |
| S3b · crecer solo | el vigilante, el reparto 60/45, bajar a los 10 min | `main` |
| S4a · prever lo ya visto | `prevision.rs`; *de un golpe* sólo hasta lo previsto; recordar tras reiniciar | 7e80f48e, 5dc7b2ec, d726bd6b |
| S4b · prever por el plan | `Entrada.memoria`, `riesgo`, `escalon_por_el_plan`, `prever_por_el_plan` | 948f6c0f, c88af1fd |
| S6 · retirar G | sin `--query-g` en las celdas del pool | 7ecff30c |
| S5 · executions | absorbido en S2b | — |
| S1b · calentar al abrir | — | deuda (0073 §8) |

Los guiones de medida, en `pruebas-de-fuego/estructura/`.
