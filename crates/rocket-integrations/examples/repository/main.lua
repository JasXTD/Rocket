return function(ctx)
    local r = ctx.repository
    if not r then
        return ui.page{title="Repository context",children={ui.text("Repository context is off or no repository is open. Enable the read-only permission for this connection, then refresh.")}}
    end
    local rows = {{"Branch",r.branch or (r.detached and "Detached HEAD" or "Unborn")},
        {"HEAD",r.head or "No commits"},{"Selected commit",r.selected_commit or "None"},
        {"Bare repository",tostring(r.bare)}}
    if r.changes then
        for _,key in ipairs({"staged","unstaged","untracked","conflicted"}) do rows[#rows+1]={key,tostring(r.changes[key])} end
    end
    return ui.page{title="Repository snapshot",children={ui.badge{text="Read only",tone="success"},
        ui.text("Captured when this action started. Refresh to observe external changes."),
        ui.table{columns={"Field","Value"},rows=rows},ui.button{id="refresh",label="Refresh snapshot"}}}
end
