# El gobierno de lo escrito — el brief desechable de W3.7 gobierno

> Se borra cuando el paso ⑥ cierre; lo que quede se dice en
> [`0031`](decisions/0031-el-puesto.md) («La decisión de W3.7 gobierno» y «Lo construido»).
> Aquí sólo lo que hace falta para construirlo en orden: qué toca cada paso, dónde, con qué
> prueba, y qué número de la medida cambia.

**La medida que manda**: `pruebas-de-fuego/medida-w3-gobierno.py` (2026-09-22). Cada paso se
cierra cuando su fila de la medida cambia como dice la tabla de 0031, y no antes.

**Lo que no se toca**: ore-iam (a qué equipo pertenece una persona: otro producto); el
vocabulario de conductos de OOS (`contextSurface` ya existe; `workspace` es una instancia);
la forma del puntero (la procedencia ya está); la consola más allá de publicar desde el puesto.

---

## ① La puerta del puesto — «desde un puesto sólo entran los verbos» · hecho (2026-09-22)

Lo que quedó: `rutas.rs::puerta_del_agente` decide por el **sujeto** (agente) y no por la
cabecera —quitarla desde la celda dejaba el mismo testigo—; leer sigue abierto; `DELETE
/documentos/Dataset` retira el puntero; `el-puesto.sh` 12. Lo que salió: `la-escritura-en-demo.py`
limpiaba por `DELETE /arbol` con el agente y desde hoy sería 403 —si se vuelve a correr, la
limpieza va por git con el testigo de la forja, que el Job ya tiene—.

**Dónde**: `crates/ore-serve/src/rutas.rs` (`con_sujeto`), `puestos.rs`, `documentos.rs`.

- Un sujeto agente con `x-ore-puesto` sólo alcanza: `GET /puestos/{id}/…` (lo suyo),
  `/v1/…` (el catálogo), `POST /datasets/…/confirmar`, `PUT|DELETE /documentos/…`,
  `/conceptos`. Todo lo demás con esa cabecera: **403** «desde un puesto sólo entran los
  verbos: leer, escribir, declarar». Decidirlo en un sitio (`Servidor::puerta_del_puesto`,
  antes del `match`), no ruta a ruta: el verbo que se añada mañana llega negado (P4, como
  `mando.rs`).
- `DELETE /documentos/Dataset/{ns}/{n}` retira también `datasets/<ns>_<n>.json` en el mismo
  commit (hoy queda huérfano). Los bytes los expira `--recoger` como siempre.
- **Prueba**: `el-puesto.sh` caso nuevo: desde la celda, `PUT /arbol/conduits.yaml` 403,
  `POST /ramas` 403, `POST /trabajos` 403 (ya), `DELETE /documentos/Dataset` deja el árbol sin
  puntero. **Medida** §2: `PUT /arbol` 200/200 → 403/403; «puntero: sí» → «no». §4:
  `PUT /arbol` desde el transform 201 → 403.

## ② El conducto de la lectura — `contextSurface.workspace` · hecho en ore-serve (2026-09-22); ②b pendiente

Lo que quedó: `flow::carga_de` + `flow::fugas` (extraídas de `vistas_materializadas`, con la
vía nueva de las columnas de la `Table` raíz) y `flow::lectura_desde_puesto`; `datos_del_puesto`
niega con el código; el índice clasifica View y Dataset por su carga; los tres SDK dicen la
frase; `el-puesto.sh` 13. Lo que salió: **sin `contextSurface.workspace`, se coteja con
`materialization.payload`** (nada cambia en demo/victor hasta que declaren un retículo).

> ✅ **②b · hecho en demo (2026-09-22)**; en victor, la condición IAM espera go (sus
> puestos abiertos leen con el token del pod y se romperían hasta reabrirse). Lo que quedó:
> `prestar {modo: leer}`, `--prestar --leer`, `datos` con `credencial`, los tres SDK con un
> secreto por raíz y `scope`, la capa por su nombre (una condición IAM sobre el nombre no da
> `objects.list`), el aprovisionador con la condición, `empujar-plantilla.py` (demo y victor
> ya llevan la plantilla nueva). Medido: STS 50–60 ms, DuckDB con la prestada 558 ms/4 filas,
> el pod bajo la condición 403 a todo menos la capa por su nombre.
>
> Lo que era: en el clúster, `over()` lee el
> bucket **con el token del pod** (W3.5b, camino (b): la cuenta `puesto` es `objectViewer` del
> bucket entero), así que ② sólo gobierna a quien pasa por `datos`; la celda puede pedir el
> objeto a GCS por su cuenta con un `metadata_location` que `GET /arbol/datasets/…json` le da.
> Lo que hay que hacer: `datos` presta una credencial CAB `objectViewer` acotada a
> `ore/v2/datasets/<p>_<t>/` (`ore-store prestar`, lo que `/v1` ya hace al escribir), los tres
> SDK leen con ella (`_iceberg`: el secreto de DuckDB con la prestada y no con la del pod;
> el sobre heredado `clave`, igual), y la cuenta del puesto **deja de ver el bucket**
> (`aprovisionar-inquilino.sh`: quitar `objectViewer` de `ore-puesto-<n>`). Se mide antes en
> `t-demo` con `jobs-p` 0 → 1 → 0 (con go): que un puesto lee con la prestada y que, sin
> `objectViewer`, el token del pod no lee nada. Es el mismo movimiento que «fuera del verbo (b)»
> hizo con la escritura.

**Dónde**: `crates/ore-core/src/assets.rs` → la clasificación efectiva se mueve a
`ore_core::clasificacion` (la usan el índice y ore-serve); `crates/ore-serve/src/puestos.rs`
(`datos_de`); `copia.rs` (`autorizar_conducto`); `puesto/*/ore` (el error en la celda).

- `autorizar_conducto` hace nacer las **dos** instancias con el dueño del paquete:
  `materialization.payload` y `contextSurface.workspace`, ambas `{ oos.maturity: DRAFT }`.
  Idempotente; un `conduits.yaml` que ya tiene una y no la otra gana la que falta.
- `datos_de(raiz, ns, nombre)` calcula la clasificación efectiva del dataset (columnas de la
  raíz, Entities que lo respaldan, y —tras ③— su procedencia) y la coteja con
  `contextSurface.workspace` por `flow::check`. Lo que no cabe: **403** con `OOS4002`, la
  etiqueta y el nivel que el conducto admite. Sin retículo declarado no hay etiqueta y nada
  cambia para los árboles de hoy.
- El SDK (los tres) lo dice en la celda tal cual: «`ventas.salida` lleva
  `gdpr.sensitivity: high` y `contextSurface.workspace` admite `low` (OOS4002)».
- **Prueba**: `el-puesto.sh`: un árbol con retículo y Entity `high` sobre lo escrito; `over()`
  403 con el código; se ensancha el conducto por el árbol (no desde el puesto) y `over()` lee.
  **Medida** §1: «la columna high, entera» → 403 OOS4002; `datos` trae `clasificacion`.

## ③ La clasificación por el grafo, hasta lo escrito · hecho (2026-09-22)

Lo que quedó: `derivedFrom` en el Dataset escrito (OOS `5d54854`: spec, esquema, tres casos de
conformance); `ore datasets --commit` lo escribe de la procedencia (`Leyo`: sobrescribir dice de
nuevo, anexar/upsert suman; nunca él mismo; sólo lo que el árbol tiene); `flow::carga_de` vía 3
(recursiva, con guarda); el índice saca `sale_de` del documento; los tres SDK quitan el propio
nombre de `leidas`; `el-puesto.sh` 14. Lo que cambió respecto al plan: **el compilador no lee
punteros** —la carga se ve con el árbol solo, y por eso va en el documento—, y `leidas` sigue
siendo la sesión entera fuera de un transform (sobreaproximar es P4). Lo que no baja: `reads` de
una `Function` (conducto `datasource`, 0029: otra costura).

**Dónde**: `C:\oos` `spec/v1alpha12/01-dataset.md` §5 (una frase y un ejemplo; bump de
`vendor/oos`); `crates/ore-core/src/{flow,assets,validate}.rs`; `puesto/*/ore` (`leidas`).

- OOS: «en un dataset escrito, la clasificación es la de su procedencia: el join de las
  efectivas de `inputs` (un transform) o de `leidas` (una sesión), sin el propio nombre». Un
  dataset escrito sin procedencia (uno de fuera, por el catálogo REST) no clasifica nada: se
  dice.
- ORE: `validate` lee los punteros `datasets/*.json` (hoy no) sólo para esto; la clasificación
  efectiva sigue `sale_de` con guarda de ciclo; el índice la enseña en `acceso.clasificacion`
  del dataset escrito.
- SDK: `leidas` se **vacía al entrar** en un transform y se rehace con sus `inputs`; fuera de
  él, quita el nombre que se está escribiendo.
- **Prueba**: conformance de OOS (un caso `valid` y uno `invalid` en `v1alpha12`);
  `ore-core/tests/assets.rs` (el derivado hereda `high`); `el-puesto.sh` (una copia mantenida
  sobre lo derivado es OOS4002 con el conducto `low`). **Medida** §5: «compila» → OOS4002;
  `{}` → `{"gdpr.sensitivity": "high"}`; la cadena sin ciclo.

## ④ Quién reescribe, y la rama · hecho (2026-09-22)

Lo que quedó: 403 (`ForbiddenException`, código 77 de `ore`) en la credencial, en el commit y
en retirar, decidido por `escrito_por`; `rama_del_puesto` + `Forja::asegurar_rama` (por git,
sin API; el trabajo igual; un directorio no tiene ramas); «Publish» en la fila de la sesión
(rubix-platform `a8e406c`, local); `el-puesto.sh` 15, `git.rs`. Lo que cambió respecto al plan:
**403 y no 409** (no es un conflicto; no es suyo). Lo que salió: una rama nace de `main` y no lo
sigue después —publicar y volver a abrir es cómo se recoge lo nuevo—; y la medida necesita
«publicar» (merge por git) entre lo de ana y lo de bob, porque ya no se ven sin eso.

**Dónde**: `crates/ore-cli/src/datasets.rs` (`--commit`, `--confirmar`), `crates/ore-serve/src/
{catalogo,puestos,documentos}.rs`, `puesto/*/ore`, rubix-platform (publicar desde el puesto).

- `ore datasets --commit`: si el puntero vigente tiene `escrito_por` y no es el sujeto, **409**
  con quién (`sobrescribir`, `anexar`, `upsert`); retirar, lo mismo por `/documentos`. El
  primer escritor de un dataset queda como suyo; una propuesta aceptada en `main` lo puede
  cambiar (es un commit de otro).
- `POST /puestos` sin `rama`: el puesto nace en `<persona>/puesto` (se crea si no está, desde
  `main`); `GET /puestos/{id}` la dice. Lo que la rama no tiene se lee de `main` (③ de W3.7,
  ya). Publicar = `POST /propuestas` desde la rama del puesto: la consola gana el botón en la
  sesión (la ficha ya sabe de propuestas, 0030 W2).
- Redeclarar lo de otro por `/documentos` no se niega: va a la rama, y la propuesta lo enseña.
- **Prueba**: `el-puesto.sh`: bob 409 sobre lo de ana; el puesto sin rama escribe en
  `persona:ana/puesto` y `main` no lo ve hasta la propuesta. **Medida** §2: «bob anexa … 200»
  → 409 con `persona:ana`; «en main» → «en bob/puesto». §3: la Entity desclasificada queda en
  la rama de bob y el índice de `main` sigue `high`.

## ⑤ Lo declarado, en el servidor

**Dónde**: `crates/ore-serve/src/puestos.rs`, `catalogo.rs`; `puesto/*/ore` (`transform`).

- `POST /puestos/{id}/transform {nombre, inputs, output}` y `DELETE …/transform` (sólo el
  agente del puesto); mientras está: `datos_de` sólo resuelve `inputs` (403 «no está en los
  inputs de `x`»), el catálogo sólo carga/confirma `output` (403). El SDK los llama al entrar
  y al salir del decorador, y el error que ya da queda como está (llega antes).
- Un trabajo lo lleva: el informe `trabajos/<id>.json` gana `declarado: {inputs, output}`.
- **Prueba**: `el-puesto.sh` 10/8/9: desde dentro, `ore.puesto.pedir(GET …/datos/otro)` es
  403; fuera del decorador vuelve a ser 200. **Medida** §4: `[PermissionError, 200, 200, 3]`
  → `[PermissionError, 403, 403, 3]`; `GET /puestos/{id}` dice el transform.

## ⑥ Medido de nuevo, y 0031

`medida-w3-gobierno.py` entera; los números en 0031 («Lo construido para W3.7 gobierno»);
la fila W3.7 de los peldaños dice «gobierno hecho»; este brief se borra.
