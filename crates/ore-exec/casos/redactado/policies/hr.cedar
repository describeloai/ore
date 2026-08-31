// La mascara mas fuerte: `redact` no transforma el valor — hace que la columna
// NO SE PIDA. Aplicarla despues seria haberlo traido.
@id("analyst-reads-redacted")
@oosMask("eu.min#sin-dni")
permit (
    principal in Role::"analyst",
    action == Action::"read",
    resource in Label::"gdpr.sensitivity:high"
) when { context.purpose == "compensation_review" };
