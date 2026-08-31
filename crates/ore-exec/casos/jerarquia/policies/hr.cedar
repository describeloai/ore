// La cadena de mando del lado del PRINCIPAL: «el que pregunta esta bajo esa
// cadena». Es expresable —`in` en Cedar es alcanzabilidad transitiva— y exige
// que el almacen lleve las aristas del principal, que es el indice de
// topologia. Sin el, esto evalua a falso y deniega sin decir por que.
@id("under-the-ceo-reads-comp")
permit (
    principal in Employee::"ceo",
    action == Action::"read",
    resource in Label::"gdpr.sensitivity:high"
)
when { context.purpose == "compensation_review" };
