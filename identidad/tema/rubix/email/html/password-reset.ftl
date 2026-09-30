<#ftl output_format="HTML">
<#--
  ═══════════════════════════════════════════════════════════════════
  EL CORREO DE REPOSICIÓN DE CONTRASEÑA

  ── ⛔⛔ POR QUÉ CADA VARIABLE VA CON `??` ───────────────────────────

    FreeMarker **aborta el envío entero** si una variable no existe, y el síntoma es un 500
    en el IdP y un correo que no llega — sin decir cuál faltaba. ⇒ todo lo que no sea
    imprescindible se consulta con `??` y degrada a un texto sensato.

    ⭐ `${link}` es la ÚNICA que no lleva salvavidas, y a propósito: sin enlace este correo
      no tiene razón de existir, así que es mejor que falle a que salga vacío.

  ── ⚠️ POR QUÉ TABLAS Y ESTILOS EN LÍNEA, en 2026 ───────────────────

    No es descuido ni nostalgia: los clientes de correo **borran las hojas de estilo** y
    muchos no aplican `flex` ni `grid`. Una maquetación moderna se ve perfecta en el
    navegador y se descuadra en Outlook — que es justo donde no se puede depurar.

  ── ⛔ Y NO HAY IMÁGENES ────────────────────────────────────────────

    Ni logo remoto ni píxel de seguimiento. Un `<img>` externo se bloquea por defecto en la
    mayoría de clientes ⇒ la marca se vería rota justo en el correo que pide confianza. La
    marca es tipográfica, que siempre se ve.
  ═══════════════════════════════════════════════════════════════════
-->
<html>
<body style="margin:0;padding:0;background-color:#f6f7f9;">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"
       style="background-color:#f6f7f9;padding:40px 16px;">
  <tr>
    <td align="center">
      <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"
             style="max-width:520px;background-color:#ffffff;border-radius:12px;
                    border:1px solid #e5e7eb;padding:40px 40px 32px 40px;
                    font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;">

        <#-- LA MARCA — tipográfica, para que no dependa de que carguen las imágenes -->
        <tr>
          <td style="padding-bottom:28px;">
            <span style="font-size:20px;font-weight:700;letter-spacing:-0.3px;color:#111827;">
              <#if realmName??>${realmName}<#else>Rubix</#if>
            </span>
          </td>
        </tr>

        <tr>
          <td style="font-size:22px;line-height:30px;font-weight:600;color:#111827;padding-bottom:16px;">
            ${msg("passwordResetTitulo")}
          </td>
        </tr>

        <tr>
          <td style="font-size:15px;line-height:24px;color:#374151;padding-bottom:28px;">
            <#if user?? && user.email??>
              ${msg("passwordResetIntroCon", user.email)}
            <#else>
              ${msg("passwordResetIntroSin")}
            </#if>
          </td>
        </tr>

        <#-- ⭐ EL BOTÓN. Va en su propia tabla porque un `<a>` con relleno se descuadra en
             Outlook, que ignora el `padding` de los enlaces. -->
        <tr>
          <td style="padding-bottom:28px;">
            <table role="presentation" cellpadding="0" cellspacing="0" border="0">
              <tr>
                <td align="center" bgcolor="#111827" style="border-radius:8px;">
                  <a href="${link}" target="_blank"
                     style="display:inline-block;padding:13px 26px;font-size:15px;font-weight:600;
                            color:#ffffff;text-decoration:none;border-radius:8px;">
                    ${msg("passwordResetBoton")}
                  </a>
                </td>
              </tr>
            </table>
          </td>
        </tr>

        <#-- ⚠️ La caducidad se dice SIEMPRE. Sin ella, quien abra el correo tarde ve una
             página de error sin entender por qué — y vuelve a pedirlo, y vuelve a tardar. -->
        <#if linkExpirationFormatter?? && linkExpiration??>
          <tr>
            <td style="font-size:14px;line-height:22px;color:#6b7280;padding-bottom:8px;">
              ${msg("passwordResetCaduca", linkExpirationFormatter(linkExpiration))}
            </td>
          </tr>
        </#if>

        <tr>
          <td style="font-size:14px;line-height:22px;color:#6b7280;padding-bottom:28px;">
            ${msg("passwordResetIgnorar")}
          </td>
        </tr>

        <#-- El enlace en texto, por si el botón no se puede pulsar (clientes que degradan a
             texto plano, o quien reenvía el correo). -->
        <tr>
          <td style="border-top:1px solid #e5e7eb;padding-top:24px;
                     font-size:12px;line-height:20px;color:#9ca3af;word-break:break-all;">
            ${msg("passwordResetEnlaceLiteral")}<br>
            <a href="${link}" style="color:#6b7280;">${link}</a>
          </td>
        </tr>

      </table>

      <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0"
             style="max-width:520px;padding-top:20px;
                    font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif;">
        <tr>
          <td align="center" style="font-size:12px;line-height:18px;color:#9ca3af;">
            ${msg("piePaladio")}
          </td>
        </tr>
      </table>

    </td>
  </tr>
</table>
</body>
</html>
