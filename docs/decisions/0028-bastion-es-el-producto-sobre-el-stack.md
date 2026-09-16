# 0028 · Bastion es el producto sobre el stack

**Estado:** aceptado (decidido el 2026-09-14 tras medir; escrito el 2026-09-16) · **Fecha:** 2026-09-16 · **Decide:** que la
inferencia de la plataforma **no es un motor propio** sino **la capa de producto que hace
soberano, multi-tenant y gestionado el stack que ya existe** —vLLM (y SGLang cuando esté
medido) sobre RTX PRO 6000 GDDR7—; que la optimización real vive **abajo** (física del hardware
y motor ajeno) y Bastion **la selecciona, la certifica, la hace reproducible y la envuelve**;
que el motor propio construido en P1–P3 queda **congelado como referencia certificada**; y que
cada máquina lleva **una etiqueta de soberanía** que el gateway respeta. Es el punto de partida
de la construcción del producto y el sujeto del `kind: Model` de [`0027`](0027-el-modelo-vive-en-el-arbol.md).

---

## El problema

Tres días de motor propio (HAL, CUDA, kernels de MoE, routing en dispositivo, CUDA graphs) y un
día de medir el estado del arte dejaron dos números que no admiten discusión: el motor propio
va **1.6× por detrás de vLLM** en la misma GPU con el mismo modelo, y la tesis del "75 % más
barato" con los expertos en RAM sale a **26–46 $ por millón de tokens**, diez veces peor que
el mismo modelo en GPU. Mientras tanto, software que ya existe sobre hardware que ya se vende
da coste de API con siete veces menos capex. Había que decidir qué es Bastion.

Lo que sigue es, literal, el documento de imagen y visión con el que se tomó la decisión.

---

## 1 · La imagen del producto

**Bastion es la capa de producto que hace soberano, multi-tenant y gestionado el stack de
inferencia que ya existe.** No es un motor. Coordina vLLM (y SGLang cuando esté medido) sobre
hardware de memoria GDDR7 barata, y lo entrega a cada cliente como un servicio propio, en su
jurisdicción, a precio de API.

La optimización real —el coste por token— vive abajo, en la física del hardware y en un motor
que otros ya escribieron. Bastion no la duplica: la selecciona, la certifica, la hace
reproducible y la envuelve en lo que una empresa europea necesita para confiarle sus datos y sus
pesos.

```
CLIENTE      un namespace por cliente en el clúster de Rubix · API compatible OpenAI /v1
             · modelos abiertos de frontera · datos y pesos en la UE
                                   ↓ lo que Bastion añade
BASTION      B1 Lanzador        máquina + perfil certificado → servidor sano → apagado verificado
             B2 Certificación   motor × modelo × máquina con tok/s, TTFT y $/M medidos: la garantía "corre impecable"
             B3 Gateway         multi-tenant: claves, contabilidad por cliente, límites, etiqueta de soberanía
             B4 Modelos         pesos verificados por hash y firma; catálogo e importación
             B5 Desbordamiento  expertos en RAM/NVMe para lo que no cabe en GPU (la única pieza propia)
             B6 Soberanía       sin Python en el perímetro, VPC, aire aislado, licencias por huella
                                   ↓ sobre
EL STACK     MOTOR    vLLM 0.29 (CUDA graphs, batching continuo, MoE FP8) · SGLang por medir
IRREMPLAZABLE MÁQUINA RTX PRO 6000 Blackwell 96 GB GDDR7 · g1 / g4 / g8 = 1 / 4 / 8 GPUs
             DÓNDE    GCP G4 europe-west1 (clientes) · bare metal ~35 k$ · Vast solo uso interno
```

**El número que sostiene la tesis** (medido el 14 sept): Qwen3-235B-A22B FP8 servido por vLLM
en 4 × RTX PRO 6000 a 32 usuarios da **587 tok/s agregados = 2.8 $ por millón de tokens**, con
TTFT de 1.4 s. Es el rango de la API de DeepSeek (2.2 $) y del nodo H100 (3–6 $), con **7×
menos capex**. Lo consigue software que ya existe; lo que falta es el producto que lo entrega.

## 2 · Dónde estamos hoy

Tras tres días de motor propio y un día de medir el estado del arte, el 14 de septiembre se
tomó la decisión de arriba. Desde entonces se construye de abajo arriba: primero el suelo,
luego la primera capa.

| Pieza | Estado | Detalle |
|---|---|---|
| Decisión: producto sobre el stack, no motor | hecho | bastion `docs/02 §3b`. El motor propio (P1–P3) queda congelado como referencia certificada. |
| Entorno base (`docs/10`) | en validación | Imagen fijada (vLLM 0.29, torch 2.13 cu130, arreglos sm_120), publicada en Artifact Registry europe-west1 como `bastion/env:0.29.0-sm120.1`, verificada sin GPU. La validación con GPU está corriendo en un g1. |
| Perfiles certificados | 3 | g4 Qwen3-235B FP8 (587 tok/s), g4 R1-0528 AWQ (147, justo), g1 V2-Lite (1 507). Cada uno con sus argumentos exactos y números esperados. |
| B1 · Lanzador | I2 en curso | I1 hecha: `bastion launch/bench/status/down/offers`, Vast y GCP, apagado verificado. I2 (ejecución real) en su cuarto intento; los anteriores enseñaron arreglos que ya están horneados. |
| Cuota GCP G4 | pendiente de Google | Pedidas preemptible 8 GPUs, 400 vCPU, 2 TB SSD en europe-west1. La bajo demanda no está expuesta al proyecto: va por ventas. |
| B2 – B6 | no empezadas | B2 nace de los perfiles; B3 es la siguiente con más valor y no necesita GPU. |
| Dinero | 2.97 $ en Vast | Suficiente para la validación g1. El g4 (~10 $) y todo lo demás espera crédito o cuota. |

**Lo medido, que es lo que vale** (tok/s agregados; prompts de 1 024 tokens y 128 generados;
$/M al precio de la máquina medida, 5.87 $/h el g4 en Vast):

| Configuración | 1 usuario | 8 | 16 | 32 | $/M a 32 |
|---|---:|---:|---:|---:|---:|
| Qwen3-235B FP8 · vLLM · 4 × RTX PRO 6000 | 76 | 270 | 429 | 587 | 2.8 $ |
| DeepSeek-R1 AWQ · vLLM · 4 × RTX PRO 6000 (eager) | 10.6 | 76 | 147 | — | 11 $ (a 16) |
| V2-Lite bf16 · vLLM · 1 GPU | 209 | 607 | 1 058 | 1 507 | 0.3 $ |
| V2-Lite · motor propio Bastion · 1 GPU | 131 | 387 | — | — | — |
| R1 con expertos en RAM (tier S, 1 GPU, llama.cpp) | 12 | — | — | — | 26–46 $ |

La fila del motor propio y la del tier de RAM son las que cerraron la vía "motor": 1.6× por
detrás de vLLM en GPU, y 10× más caro por token con los expertos fuera de la GPU.

## 3 · La visión

**Para quién.** Empresas europeas que quieren modelos abiertos de frontera —235B, 671B— con
datos, prompts y pesos en su jurisdicción, sin pagar el nodo H100 ni renunciar al precio de API.

**La promesa.** *Elige modelo y máquina; corre impecable, soberano, a precio de API.* La matriz
de certificación es la garantía: cada par modelo × máquina que ofrecemos tiene un número medido
detrás, no una estimación.

**Cómo se cobra.** Máquina dedicada a precio fijo por hora con tokens ilimitados (la forma que
Scaleway ya vende sobre H100), y serverless por token compartido cuando haya volumen. La
diferencia estructural: **hardware ~5× más barato por token que Hopper** para MoE grandes, con
el mismo motor.

**Frente a quién.**
- **Scaleway / nubes UE**: mismo enfoque (vLLM gestionado, soberano) sobre H100 a 30 €/h para
  un MoE grande. Ganamos en coste por token y capex; hay que igualar su capa de producto.
- **APIs públicas** (DeepSeek 2.2 $/M): precio similar, cero soberanía.
- **Nodo H100 propio**: 250 k$ de capex por lo que una caja de 35 k$ hace para este tamaño de
  modelo.

**Lo que no somos.**
- No un motor: no competimos con vLLM/SGLang, los certificamos.
- No un host comunitario para clientes: Vast y similares son solo para validar y certificar.
  Cada máquina lleva su etiqueta de soberanía y el gateway la respeta.

**Hitos, en orden.**
1. **La imagen pasa con GPU.** Un `PASS` de `bench.sh` con la imagen publicada, lanzado por
   `bastion`, informe en `docs/runs/`. Cierra el entorno base. Esta semana, con 3 $.
2. **Primer g4 en GCP.** Cuando llegue la cuota: el mismo comando con `--provider gcp`. El número
   de 2.8 $/M reproducido en la nube donde viven los clientes.
3. **Gateway multi-tenant sobre un g4.** B3: claves, contabilidad y etiqueta de soberanía por
   namespace. Es el momento en que un cliente de Rubix puede usar Bastion.
4. **Matriz de certificación publicada.** B2: motor × modelo × máquina con $/M. La página que
   sustituye al argumento de venta.
5. **R1 completo y segundo motor.** g8 (8 GPUs) para 671B con sitio para CUDA graphs; SGLang
   medido frente a vLLM en la misma máquina; el tier de desbordamiento (B5) donde compense.

---

## Lo que se acepta a cambio

- **Tres días de motor que no van al producto.** HAL, backend CUDA, kernels de MoE, routing en
  dispositivo y CUDA graphs quedan como referencia certificada y como el backend del tier de
  desbordamiento (B5), no como el camino al coste por token. Se congelan, no se borran.
- **Dependencia de un motor ajeno.** vLLM cambia de versión cada pocas semanas y cada versión
  puede romper una GPU (en sm_120 hicieron falta cuatro arreglos a mano). Es exactamente el
  trabajo que se vende —la certificación—, pero es trabajo que no termina.
- **La afirmación comercial cambia de forma.** No "75 % más barato que la GPU" sino "coste de
  API con 7× menos capex, soberano". Es menor y es cierta.
- **Sin cuota de G4 no hay cliente.** La forma soberana vive en GCP G4 y la cuota bajo demanda
  no está expuesta al proyecto. Hasta que llegue, todo lo real se valida en un mercado
  comunitario que no puede tocar datos de clientes.
- **Una imagen de 27 GB.** torch cu130 y sus librerías CUDA. Cada máquina nueva la baja; en
  GCP dentro de la región es gratis, fuera cuesta ~3 $ por máquina.

---

## Lo que este abordaje NO hace, y por qué

- **No reemplaza a vLLM/SGLang.** Los certifica. El día que un motor mejor aparezca, es un
  perfil nuevo en la matriz, no una reescritura.
- **No sirve clientes en hosts comunitarios.** La etiqueta `community, no-guarantee` existe
  para que el gateway no pueda enrutar un tenant soberano allí ni por error.
- **No decide el segundo motor ni el tier de desbordamiento.** SGLang y B5 se miden antes de
  entrar; hoy son huecos con su sitio.
- **No promete el número de GCP.** El 2.8 $/M está medido en Vast con el mismo hardware; en
  G4 se reproduce con el mismo comando el día que haya cuota, y hasta entonces se dice así.
