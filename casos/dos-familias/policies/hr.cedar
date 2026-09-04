@id("analyst-reads")
permit (
    principal in Role::"analyst",
    action == Action::"read",
    resource in Label::"gdpr.sensitivity:high"
) when { context.purpose == "compensation_review" };

// La clave, explicitamente. ④ la necesita para ensamblar, y pedirla sin
// autorizarla seria devolver un dato que ① no permitio — asi que se autoriza
// aqui, a la vista, en vez de colarla en la proyeccion por conveniencia.
@id("analyst-reads-the-key")
permit (
    principal in Role::"analyst",
    action == Action::"read",
    resource == Property::"hr.Employee.employeeId"
) when { context.purpose == "compensation_review" };
