<#ftl output_format="plainText">
<#--
  ⭐⭐ LA VERSIÓN EN TEXTO PLANO — y no es opcional.

    Keycloak manda el correo en `multipart/alternative`: si esta mitad no existe, algunos
    clientes muestran el HTML crudo con las etiquetas a la vista. Y los filtros de spam
    puntúan PEOR un mensaje que sólo trae HTML.

  ⛔ Mismo contenido, mismas claves de mensaje. Que las dos mitades digan cosas distintas es
    la forma más fácil de que alguien lea una promesa que la otra no cumple.
-->
${msg("passwordResetTitulo")}

<#if user?? && user.email??>${msg("passwordResetIntroCon", user.email)}<#else>${msg("passwordResetIntroSin")}</#if>

${msg("passwordResetBoton")}:
${link}

<#if linkExpirationFormatter?? && linkExpiration??>${msg("passwordResetCaduca", linkExpirationFormatter(linkExpiration))}
</#if>
${msg("passwordResetIgnorar")}

--
${msg("piePaladio")}
