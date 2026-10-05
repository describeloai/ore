# 0054 · Converger sin romper

**Estado:** **aceptado · en vivo** (2026-10-05) · R0, R1 y R2 hechos y en vivo; R3 pendiente. **Decide:** qué puede y
qué no puede hacer un reconciliador que corre solo, y cómo se rota una credencial de la que vive un
servicio, para que **un fallo transitorio no pueda dejar a un inquilino sin su base**. Nace de un
incidente medido (abajo). Toca [`0047`](0047-ore-access-control.md) A7a (el papel de cada celda) y
[`0022`](0022-el-inquilino-es-un-repositorio.md) (el aprovisionador).

## El incidente (2026-10-04/05), medido

| cuándo (UTC) | qué | de dónde |
|---|---|---|
| 04-oct 21:35 y 21:38 | el `CronJob` `aprovisionador` (cada 5 min) **rota** la clave de `cofre_demo` y `cofre_victor` | `iam.huella`, `celda:papel-de-base`, `rotada: true`, `quien: aprovisionador` |
| — | y **no la guarda**: ninguna `AddSecretVersion`; las versiones vivas de `t-<n>-base-del-cofre` siguen siendo las del 28-sep | registro de auditoría y `gcloud secrets versions list` |
| 21:35 | esa pasada tampoco dejó sus `SetIamPolicy`, que el resto deja cada ~4 min: **una pasada en la que gcloud falló entera** | registro de auditoría |
| 04-oct → 05-oct 05:53 | nada se nota: los custodios vivos tienen la conexión abierta y no vuelven a autenticarse | `ore-cofre` guarda **un** `Client` toda su vida |
| 05-oct 05:53 | el nodo se reinicia; los custodios arrancan con la clave del secreto y la base la rechaza | `password authentication failed for user "cofre_victor"` |
| 05:53 → | **t-victor sin custodio**, en `CrashLoopBackOff`, sin que nadie lo vea | `kubectl get pods` |
| 07:31 | un despliegue rehace el de t-demo (`Recreate`): el viejo se va antes de que el nuevo esté sano, y t-demo cae también | el job `imagen` del CI |

La línea que lo hizo, en `malla/aprovisionar-inquilino.sh`:

```sh
elif [ -n "$("$GCLOUD" secrets versions list "$BASE_COFRE" … 2>/dev/null)" ]; then
  ya "la base de este cofre"
else
  … iam.dar_papel_de_celda(…)      # ROTA
```

**El error de gcloud se tiraba, y una respuesta vacía se leía como «no hay versión».**

## Las cinco causas, y cada una es una clase

1. **«No lo sé» se leía como «no existe».** 27 llamadas a gcloud en el guion tiraban su error (`2>/dev/null`, `2>&1`, `|| true`); una
   decide en la retirada qué secretos y buckets **borrar**.
2. **Un bucle que corre solo podía rotar.** Rotar es un acto; converger no debe contener actos.
3. **Rotar no era atómico**: la clave nueva nacía en la base y se guardaba después; un corte en
   medio la perdía en todos los sitios.
4. **Una sola credencial por custodio**: cambiarla deja fuera a quien usa la de antes.
5. **Nadie se entera**: 8 h latente, 2 h caído; y un despliegue que retira lo sano antes de tener
   lo nuevo.

## Las invariantes

| | invariante | dónde se hace cumplir |
|---|---|---|
| **I1** | **«No lo sé» nunca es «no existe».** Toda pregunta tiene tres respuestas: contestó, **no existe** (lo dice el proveedor: `NOT_FOUND`, `HTTPError 404`, `not found: 404`, medidos uno a uno) y **no se sabe** (cualquier otra cosa). «No se sabe» no toca nada y la pasada sale en rojo. | `preguntar` en el guion (las 27); `correr` deja en rojo una escritura que falla; `pruebas-de-fuego/converger-sin-romper.sh` en el CI prohíbe la forma y fija las tres respuestas |
| **I2** | **El reconciliador sólo crea; nunca rota.** Crear el papel de una celda que ya tiene uno es un error **de la base**, no un descuido del guion. | `iam.crear_papel_de_celda` (049) se niega si la celda tiene papel; `dar_papel_de_celda` desaparece |
| **I3** | **La versión viva del secreto abre la base en todo instante**, aunque el proceso muera en cualquier paso. Dos papeles por celda, alternos (`cofre_<c>` y `cofre_<c>_b`): se prepara el que **no** está vigente, se comprueba que entra, se guarda, el custodio arranca con él, y sólo entonces se retira el otro. | `iam.preparar_papel_de_celda` **no puede** tocar el vigente; `iam.confirmar_papel_de_celda` exige que el papel nuevo **tenga una sesión abierta** (el custodio ya entra con él) |
| **I4** | **Una credencial rota se ve en minutos**, no en el siguiente reinicio. | R3: el custodio comprueba su login de forma periódica y deja de estar listo; aviso |
| **I5** | **Un despliegue no retira un custodio sano antes de tener el nuevo.** | R3: `RollingUpdate` con `maxUnavailable: 0` (el `Recreate` era por CPU, y hoy sobra ~1770m) |

### Rotar (I3), paso a paso, y qué queda si se corta

`malla/rotar-base-del-cofre.sh <celda>`, un acto de una persona con el clúster en la mano:

| paso | qué | si se corta aquí |
|---|---|---|
| 1 | `preparar`: clave nueva en el papel **no** vigente | el vigente sigue entrando; el secreto no cambió |
| 2 | se comprueba que la clave nueva entra (por la entrada estándar del pod, nunca en `argv`) | igual |
| 3 | versión nueva del secreto | las dos claves entran: el custodio entra con cualquiera |
| 4 | se reinicia el custodio y se espera a que esté listo | igual |
| 5 | `confirmar`: sólo si el papel nuevo tiene una sesión abierta; marca vigente y deja el otro sin login | — |
| 6 | se deshabilitan las versiones viejas del secreto | sólo higiene |

La clave no pasa nunca por la salida, ni por `argv`, ni por la huella.

**La primera rotación es la recuperación del incidente**: no hay un parche aparte; el mecanismo
bueno es el que levanta t-demo y t-victor.

## Lo que R1 destapó

- **Dos pasos fallaban en silencio en cada pasada**, ahora mismo: `iam roles describe oreTocarBlobs`
  (el papel del aprovisionador no tiene `iam.roles.get`: se leía «no existe» e intentaba crearlo, y
  el `create` también fallaba) y `services identity create` (sin `serviceusage`, tapado con
  `|| true`). Son de plataforma: dentro se dicen y no se intentan; desde fuera, con tres respuestas.
- **La prueba encontró dos que la búsqueda a ojo no vio** (la llave y el `uniqueId` del puente),
  partidas en varias líneas. Por eso mira líneas lógicas, no líneas.
- **El incidente, reproducido**: con un gcloud que falla sólo en `secrets versions list`, el guion
  en seco dice «NO SE SABE, y no se toca» y sale 1. El de antes habría rotado.
- Quedan fuera, dichos: los guiones de una vez que corre una persona (`68-…`, `71-…`, `72-…`) y los
  `⚠` que son estados («la forja todavía no está») y no fallos.

## Hitos

| hito | qué | estado |
|---|---|---|
| R0 | este ADR | **hecho** |
| R1 | I1 (tres respuestas en las 27 llamadas, prueba del CI) e I2 (`crear`, migración 049) | **hecho** (644d91e; 049 aplicada en vivo) |
| R2 | I3 (`preparar`/`confirmar`, `rotar-base-del-cofre.sh`); su primera corrida recupera t-demo y t-victor | **hecho**: victor → `cofre_victor_b` (secreto v2), demo → `cofre_demo_b` (v3); los viejos sin login y sus versiones deshabilitadas; los dos custodios en pie con una sesión cada uno |
| R3 | I4 (login periódico del custodio, aviso) e I5 (`RollingUpdate`) | pendiente |
