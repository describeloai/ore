# Handoff · el camino hasta un registro

> **Este documento es desechable.** Se borra el día que exista un registro que sirva un
> paquete firmado a un tercero que no somos nosotros. Un plan que sobrevive a su ejecución
> deja de ser un plan y pasa a ser documentación de un pasado que ya nadie comprueba.
>
> Fecha: 2026-09-01 · Escrito **después** de medir, no antes.

---

## 1. Dónde estamos

La cadena de distribución existe de punta a punta y **le falta el último eslabón**:

```
ore pack  →  ore-fetch  →  vendorizado  →  digest verificado  →  compila
   ✅           ✅              ✅                ✅               ✅
                │
                └── contra un DIRECTORIO. No hay registro, no hay firma, no hay actualización.
```

Lo que ya no es una promesa, y se comprueba ejecutándolo:

| | Qué está demostrado |
|---|---|
| **el contenedor no cambia la identidad** | el mismo paquete como árbol y como `.oob` digiere igual |
| **el origen no tiene que ser de confianza** | rebajar `personalEmail` de `high` a `low` dentro de un `.oob` para la compilación |
| **el registro es prescindible** | se trae un paquete, se **vacía el origen**, y el árbol sigue compilando |
| **quien pide no se cree nada** | un `.oob` que dice otro paquete no se escribe |

`v1alpha6` fija el formato y el contrato de obtención. Lo que queda **no es más
especificación**: es que actualizar una dependencia sea una decisión informada, y que alguien
pueda decir de quién es lo que usas.

### Medido hoy, no recordado

Tres cosas que no funcionan, y las tres salieron de intentar subir un vocabulario de `0.1.0` a
`0.2.0` con una clasificación elevada —`dateOfBirth` de `medium` a `high`—:

**a · `ore lock` no sube una versión: falla.**

```
$ ore lock .                       # con `^0.2` declarado y `0.1.0` vendorizado
error: `oos.dev/regulatory/gdpr` está en el árbol como `0.1.0`, y se pidió `^0.2`
```

Trae lo que **falta por nombre**, no lo que está y se quedó corto. Actualizar hoy es borrar el
`.oob` a mano.

**b · Y si no lo subes, nadie lo dice.** Con `^0.2` en el manifiesto, `0.1.0` en el lock y
`0.1.0` vendorizado:

```
$ ore validate .
ok · sin errores
```

`OOS2013` comprueba que cada dependencia declarada **esté** en el lock, y `coincide_con_el_lock`
que el árbol sea lo que el lock dice. **Nadie compara la versión resuelta con el rango
pedido.** El manifiesto pide una cosa, el lock resuelve otra, y el build sale verde.

**c · `ore diff` no lee un `.oob`.**

```
$ ore diff gdpr-0.1.0.oob gdpr-0.2.0.oob
error: `gdpr-0.1.0.oob` no es un directorio de paquete
```

Comparar lo que tienes con lo que vendría —que es la pregunta entera de una actualización— no
se puede hacer con lo que se publica.

**Y lo que sí funciona, que es la mitad buena:** `ore diff` sobre los dos árboles clasifica el
cambio **sin un solo código nuevo**.

```json
{ "axis": "CONSUMER", "code": "OOS5009",
  "subject": "gdpr.dateOfBirth", "from": "medium", "to": "high" }
"requiredBump": "major"
```

La máquina para contestar *«qué me rompe esta actualización»* ya existe. Lo que falta es
apuntarla a una dependencia.

---

## 2. Prioridades, en orden de riesgo retirado

### P0 · Que actualizar sea posible, y que no actualizar se note

Los tres defectos de arriba, que son el mismo: **el ciclo de vida de una dependencia no
existe.** Publicar y consumir por primera vez funcionan; lo que pasa después, no.

- `ore lock` trae una versión mejor cuando la del árbol no satisface el rango, y **retira la
  vieja** — dos `.oob` del mismo paquete en el árbol son dos verdades.
- Una versión resuelta fuera del rango declarado es un diagnóstico, no un silencio. Es
  `OOS2013` otra vez —el lock y el manifiesto discrepan— y no hace falta código nuevo.
- `ore diff` acepta un `.oob` donde acepta un directorio. Un paquete es un paquete.

**Listo cuando:** subir `^0.1` a `^0.2` trae la nueva, reescribe el lock, retira la vieja, y
declarar un rango que el lock no satisface **no compila**.

> **Hecho.** Una versión corta ya no es un callejón sino una dependencia por resolver: se pide,
> se verifica y se vendoriza, y la anterior se retira **por lo que su sobre dice ser** y no por
> el nombre del fichero. Un rango que el lock no satisface es `OOS2013`, sin código nuevo. Y
> `ore diff` acepta un `.oob` donde acepta un directorio, mezclados si hace falta: comparar dos
> `.oob` da **byte a byte** el mismo informe que comparar los dos árboles de los que salieron,
> que es la misma propiedad que ya tenía el digest —el contenedor no cambia la identidad—
> aplicada al veredicto. P1 ya tiene de dónde tirar.

**Por qué primero:** es la única tarea que puede cambiar todo lo demás. Un vocabulario que no
se puede actualizar no se adopta, y entonces la firma, el log y el registro decoran algo que
nadie usa.

### P1 · El impacto, que es el producto

`ore diff` dice qué cambió **en el vocabulario**. Nadie dice qué significa **en tu árbol**, y
esa es la pregunta que alguien paga por tener contestada:

> *«El artículo 9 cambió. Estas doce propiedades tuyas suben de clasificación y tres se quedan
> sin regla que las cubra.»*

La maquinaria está entera —`flow::efectivas` computa la clasificación efectiva y `governance`
sabe qué exige cobertura—: falta correrla **dos veces**, con el `.oob` viejo y con el nuevo, y
restar.

**Listo cuando:** aceptar un bump enumera las propiedades **propias** afectadas antes de
escribir el lock, no después de que falle el build.

**Aviso de alcance:** esto no es un informe bonito. Si no dice exactamente las que cambian —ni
una de más, que sería ruido, ni una de menos, que sería una fuga— no sirve, y es la misma
exigencia que el peldaño 2 de v1alpha5 le pone al techo de un conducto.

> **Hecho.** `ore lock` enumera, antes de escribir el lock, las propiedades **propias** que
> cambian de clasificación y las que se quedan sin regla que las cubra —con el porqué, tal y
> como lo nombra el compilador— y dice también cuando no cambia nada. Se computa entre los dos
> estados del **mismo árbol**, no entre las dos versiones del vocabulario, que es toda la
> diferencia entre informar y hacer ruido.
>
> Lo que lo hace fiable no es el informe: es que **no computa nada nuevo**. La exigencia de
> gobierno se extrajo de `OOS8001` a `governance::exigencias` en vez de copiarse, así que lo que
> se anuncia y lo que rompe la compilación son la misma función. Medido ejecutando las dos.

### P2 · La firma, y dónde vive cada mitad

Un digest dice **qué** es. Una firma dice **de quién**, y es lo único que un digest no puede
contestar — ni un bucket, ni un espejo, ni nosotros.

La decisión está medida, y sale de la doctrina que ya existe:

| | Dónde | Por qué |
|---|---|---|
| **verificar** | **dentro** de `ore` | es aritmética sobre bytes que ya tienes. No es red |
| **firmar** | **fuera**, delegado | exige una clave privada, y **una credencial nunca entra en el compilador** |

Es la misma frontera que `source add` traza para un secreto y que `ore-read-<tipo>` traza para
una conexión. Y el veto de `dependencias.rs` **no lo impide**: veta *red y FFI* —`ring` está
vetada por traer ensamblador, no por ser cripto— y `sha2` ya está dentro porque el digest la
necesita. Un verificador Ed25519 en Rust puro cabe; sube `CIERRE`, y eso es una decisión
deliberada que se escribe en el commit.

**Listo cuando:** el lock lleva, junto al digest, quién lo firmó; y un `.oob` cuya firma no
case **no se usa**, con el mismo trato que hoy recibe un digest que no case.

### P3 · El log de transparencia

Sin firma no significa nada, y por eso va después. Con firma, es lo que convierte *«confío en
esta clave»* en *«puedo comprobar que esta clave nunca dijo dos cosas distintas»* — la forma
que ya resolvieron la base de sumas de Go y Sigstore, y que en sector regulado es **el**
producto: poder probar qué decía la definición de dato personal en una fecha.

**Listo cuando:** un tercero replica el log y verifica la inclusión de una versión **sin
preguntarnos nada**.

### P4 · El servicio, que va el último a propósito

Un índice y blobs direccionados por contenido. Estático, replicable con un `rsync`, sin base
de datos en el camino crítico.

Va el último porque es la parte **commodity**: direccionar por contenido ya le quitó la
integridad, y P2 y P3 le quitan la autoridad. Lo que queda es servir ficheros, y eso lo hace
cualquiera — que es exactamente la propiedad que se quiere conservar.

**Listo cuando:** dos consumidores que obtienen el mismo paquete de **orígenes distintos**
compilan el mismo bundle. Es el peldaño 3 de `01-distribucion` §6, medido entre dos orígenes
de verdad y no entre dos directorios.

---

## 3. Lo que **no** entra, y no por falta de tiempo

**Cuentas, permisos y publicación autenticada.** Son del servicio, y el servicio es lo último.
Diseñarlas ahora ataría el formato a un modelo de identidad que P2 todavía no ha elegido.

**Búsqueda en el camino crítico.** Buscar, navegar y «quién depende de esto» son útiles y
**desechables**: si se cae el buscador, los builds siguen. Un registro cuyo índice puede tumbar
una compilación ha dejado de ser un registro.

**Retirar una versión como mecanismo de seguridad.** Retirar rompe a quien no la tenga, no a
quien ya la verificó — y eso es correcto. Lo que sustituye a una retirada es **una versión
nueva y un aviso**, no borrar el pasado.

**Un formato de archivo.** Ya se decidió y por escrito: un `.oob` es la forma canónica, no un
`tar.gz`. Comprimir en tránsito es de la capa de transporte.

**Red en `ore`.** No se reabre. Traer se delega, y eso es una propiedad comprobada por
`dependencias.rs`, no una promesa.

---

## 4. Deriva encontrada de paso

- ~~`ore diff` acepta directorios y **el resto del motor ya acepta un `.oob`** desde que el
  cargador lo abre. Es el único comando que se quedó atrás.~~ Resuelto en la raíz y no en el
  comando: cargar un paquete admite las dos formas, así que ninguno de los que vengan volverá
  a quedarse atrás por separado.
- ~~El aviso de `no_encontrada` en `candado.rs` enumera lo que hay en el árbol, y **no dice la
  versión** — que en un fallo de rango es justo el dato que falta.~~ Resuelto.
- La especificación dice que el contenedor no cambia la **identidad**, y no dice que tampoco
  cambia lo que una herramienta **concluye** sobre el paquete. Hoy es cierto por construcción
  —se comparan formas canónicas— y por eso vale la pena escribirlo antes de que alguien
  implemente un motor donde no lo sea.
