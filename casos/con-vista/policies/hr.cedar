@id("analyst-reads")
permit (
    principal in Role::"analyst",
    action == Action::"read",
    resource in Label::"gdpr.sensitivity:high"
) when { context.purpose == "compensation_review" };
