# -*- coding: utf-8 -*-
"""`invitar`, `admitir`, `conceder`, `revocar` — medidos antes de escribirlos.

`fundar` corre. Los cuatro que faltan tienen cada uno una pregunta que no tiene
respuesta obvia, y tres de las cuatro ya las contesto la plataforma — con la
cicatriz al lado.

  A. QUIEN PUEDE QUE       la escalera, y el rodeo que hay que cerrar
  B. EL SECRETO            el id NO es el vale
  C. EL CORREO             se normaliza; el `sub` NO
  D. EL AMBITO             el hueco de `007`, y por que no se edita
  E. REVOCAR               no borra
"""
import pathlib
import textwrap

RUBIX = pathlib.Path(r"C:\Rubix\modelo\migraciones")


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def limpio(s):
    """Sin los glifos que una consola cp1252 no sabe pintar.

    Se citan ficheros ajenos y llevan estrellas y prohibidos. Perder el glifo no
    pierde la frase; morir al imprimirla, si.
    """
    return "".join(c if ord(c) < 0x2013 else "*" for c in s)


def cita(f, aguja, cuantas=4):
    """Las `cuantas` lineas de comentario a partir de la que contiene `aguja`."""
    try:
        t = (RUBIX / f).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return []
    lineas = [l for l in t.splitlines() if l.strip().startswith("--")]
    for i, l in enumerate(lineas):
        if aguja in l:
            return [x.strip().lstrip("-").strip() for x in lineas[i:i + cuantas]]
    return []


print("== los cuatro verbos, medidos ==")

# -- A -----------------------------------------------------------------------
print()
print("A - QUIEN PUEDE QUE, Y EL RODEO")
print()
print("   papel           puede")
print("   " + "-" * 74)
for p, q in [
    ("lector", "ve"),
    ("miembro", "ve y decide sobre la ontologia"),
    ("administrador", "ademas invita, revoca y concede"),
    ("dueno", "ademas traspasa. Es UNO por organizacion"),
]:
    print("   %-15s %s" % (p, q))
print()
parrafo("La escalera es la que ya esta en `002-el-papel.sql`, y `ordinal` la "
        "ordena. Lo que falta es la guarda, y su motivo lo escribieron ellos:")
print()
for l in cita("021-la-invitacion.sql", "sin esa segunda potestad", 4):
    print("     %s" % limpio(l))
print()
parrafo("==> **Nadie puede invitar a alguien con un papel mas alto que el "
        "suyo.** Sin esa guarda, `invitar` es una escalada de privilegio con "
        "forma de cortesia: un administrador invita a un complice como dueño y "
        "tiene por un rodeo lo que no se le da de frente.")
parrafo("Y la comprobacion vale para `conceder` igual: conceder un papel que "
        "no se tiene es lo mismo por otra puerta.")

# -- B -----------------------------------------------------------------------
print()
print("B - EL SECRETO: EL `id` NO ES EL VALE")
print()
parrafo("En su tabla el `id` es la clave primaria Y lo que viaja en el enlace "
        "del correo. Eso tiene dos consecuencias que conviene no heredar:")
print()
print("     quien pueda LISTAR invitaciones puede redimirlas")
print("     un `pg_dump` es una carpeta de vales al portador")
print()
parrafo("==> Se parten en dos: un `id` publico —listable, citable en una "
        "huella— y un SECRETO que solo viaja en el correo. Y de el se guarda "
        "**el resumen, no el secreto**: la misma disciplina que una "
        "contraseña. Comprobar es resumir lo que llega y comparar.")
parrafo("`iam.invitacion` ya existe y esta APLICADA, asi que esto no se edita "
        "ahi: se escribe la migracion siguiente. Es la regla del runner "
        "cobrandose su primer uso — «corregir no es editar, es escribir la "
        "siguiente».")

# -- C -----------------------------------------------------------------------
print()
print("C - EL CORREO SE NORMALIZA. EL `sub` NO")
print()
for l in cita("021-la-invitacion.sql", "Y esto NO contradice a `014`", 4):
    print("     %s" % limpio(l))
print()
parrafo("La distincion vale entera: el `sub` es una CLAVE de identidad y "
        "plegarlo fundiria dos personas en una; el correo de una invitacion es "
        "una CITA, y dos que solo difieren en la caja serian dos vales para la "
        "misma persona.")

# -- D -----------------------------------------------------------------------
print()
print("D - LA CONCESION NO TIENE AMBITO — y es un hueco NUESTRO")
print()
parrafo("`007-la-concesion.sql` guarda `(sujeto, recurso, papel)` y **no dice "
        "de que organizacion es el recurso**. Con eso nadie puede contestar "
        "quien esta autorizado a conceder sobre el: la guarda de A necesita "
        "saber en que organizacion mirar el papel de quien concede.")
print()
print("     hoy      concesion(sujeto, recurso, papel, ...)")
print("     falta    concesion.organizacion")
print()
parrafo("Se cierra con una migracion nueva. Y NO editando la 007: esta "
        "aplicada, y el runner se niega — lo comprobamos a proposito el dia "
        "que se escribio.")

# -- E -----------------------------------------------------------------------
print()
print("E - REVOCAR NO BORRA")
print()
parrafo("`revocada_en` y `revoco`, y la fila se queda. La vista "
        "`concesion_viva` es la que se consulta; la tabla guarda tambien lo "
        "que VALIO, que es lo unico que hace auditable una revocacion.")
print()
parrafo("Y lo mismo con la invitacion: revocarla despues de redimida sigue "
        "habiendo dejado entrar a alguien. El estado se deriva de las marcas "
        "—P2— y la huella cuenta el orden.")
