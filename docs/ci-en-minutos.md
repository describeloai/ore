# El CI en minutos

**Objetivo:** de *push* a desplegado, **3 minutos** el caso normal y **5 como techo**, también cuando se
tocan binarios. Hoy son ~40 (medido el 2026-10-23 en `1cb8e0c1`).

## 1. Dónde se va hoy (`1cb8e0c1`, todos los jobs a la vez, el despliegue espera al último)

| job | dura | en qué |
|---|---|---|
| `conectores` | **37 min** | 10 min construir el laboratorio de Phoenix + 9 el de HBase (desde el archivo de Apache), y ~15 conectores **en serie** |
| `hbase-kerberos` | 19 min | un HBase con Kerberos montado de cero, en tres protecciones, **en serie** |
| `construir` | 18 min | 11,5 compilar binarios *release* en un runner de 4 vCPU, 4,3 construir y subir imágenes, 1 el frontend |
| `suite` | 4 min | 3 la suite (`cargo test`) |
| los demás | 1–4 min | |
| `imagen` | 3 min | espera a **todos** |

Y cuando uno falla se repiten los 40: el 2026-10-23 fueron tres vueltas (dos fallos de `conectores`).

## 2. Los factores

1. **Se prueba lo que no cambió.** Los laboratorios y los conectores corren en cada commit, toque o no
   lo suyo. *(Arreglado en la rama `ci-3-minutos`: huella por prueba, `cambios` + `puerta`.)*
2. **Se construye lo que no cambió.** Los laboratorios salen del archivo de Apache cada vez. *(Arreglado:
   `ci/laboratorio.sh`, por su huella en GHCR.)* Los binarios ya van por huella (0060 B2).
3. **Compilar *release* en una máquina pequeña.** 11,5 min en 4 vCPU; tocar `ore-core` recompila 22 de 24
   binarios (camino crítico `ore-code → ore-core`).
4. **Todo en serie dentro de un job.** Los conectores no dependen entre sí y van uno tras otro.
5. **Esperas fijas.** `sleep 30`, `sleep 20` tras cada laboratorio, además del sondeo.
6. **El despliegue espera a todo.** El job más lento manda aunque no tenga nada que ver con el cambio.
7. **Nadie lo vigila.** Ningún job tiene presupuesto: crecer de 8 a 37 min no rompió nada.

## 3. Cómo se llega

| paso | qué | queda en |
|---|---|---|
| **C1** | huella por prueba + laboratorios en GHCR (rama `ci-3-minutos`) | ~20 min si se toca `ore-core`; ~7 si sólo `ore-serve` |
| **C2** | `construir`: runner grande (16 vCPU) y caché de `target/` persistente por huella; compilar sólo los binarios cuya huella cambió (ya), en paralelo con las imágenes | ~5 |
| **C3** | lo que queda de `conectores` y `hbase-kerberos`, partido en jobs en paralelo (uno por conector) | cada uno ≤ 5 |
| **C4** | cortar el abanico de `ore-core`: que tocarlo no recompile 22 binarios | 3 |
| **C5** | las reglas de §4, cumplidas por el propio CI | se queda |

C2 cuesta dinero (un runner grande de GitHub, o uno propio en GKE, que choca con la cuota de 12 vCPU):
se decide aparte.

## 4. Las reglas fijas

1. **Ningún job pasa de 5 minutos.** `timeout-minutes: 5` por defecto en todos; una excepción va en una
   lista de este documento, con motivo y fecha de caducidad.
2. **Nada se construye dos veces.** Binario, imagen, laboratorio o frontend: por la huella de lo que lo
   produce, y si el registro ya la tiene, se baja.
3. **Nada se prueba si no cambió lo suyo.** Cada prueba larga tiene huella (lo que nombra: su bloque, sus
   paquetes con sus dependencias, sus rutas) y un verde por huella no se repite.
4. **El despliegue espera sólo a lo que el cambio tocó** (`puerta`).
5. **En paralelo por defecto.** Dos pasos van en serie sólo si uno usa lo que el otro deja, y se dice.
6. **Nada de esperas fijas.** Sólo sondeos de «ya está listo», con su tope.
7. **Cada CI dice cuánto tardó cada job** (resumen del run), y uno que se pasa de su presupuesto falla.
