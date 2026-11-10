-- Jira Cloud API v3. Credentials and the approved origin belong to Rocket.
-- Rich descriptions remain unchanged unless the user requests replacement.
-- Where the API starts. A token with scopes reaches it through Atlassian's gateway,
-- under the site's cloud ID; each run sets this from the connection's settings.
local rest = "/rest/api/3"
local function text(value, limit)
    if type(value) ~= "string" then return "" end
    value = value:gsub("\r\n", "\n"):gsub("[%z\1-\8\11-\31\127]", " ")
    local last = utf8.offset(value, (limit or 1024) + 1)
    return last and value:sub(1, last - 1) or value
end
local function document(value)
    if value == "" then return {type="doc",version=1,content={{type="paragraph"}}} end
    return {type="doc",version=1,content={{type="paragraph",content={{type="text",text=value}}}}}
end
local function get(path) return rocket.http{method="GET",path=path}.json end
local function issue_key(value)
    local key = text(value,48):upper()
    if key:match("^[A-Z][A-Z0-9_]*%-%d+$") then return key end
end
-- ROCKET-42, anywhere in a branch name.
local function branch_key(branch)
    for key in text(branch,1024):gmatch("%f[%w]%u[%u%d_]*%-%d+") do return key end
end
local function plain(node, depth)
    if type(node) ~= "table" or depth > 10 then return "" end
    if node.type == "text" then return text(node.text,1024) end
    if node.type == "hardBreak" then return "\n" end
    local parts = {}
    for i,child in ipairs(node.content or {}) do
        if i > 100 then break end
        parts[#parts+1] = plain(child,depth+1)
    end
    return text(table.concat(parts,node.type == "doc" and "\n" or ""),1024)
end
local function when(value)
    return (text(value,16):gsub("T"," "))
end
local function can_set(meta, name)
    local field = (meta.fields or {})[name]
    if type(field) ~= "table" then return false end
    for _,operation in ipairs(field.operations or {}) do if operation == "set" then return true end end
    return false
end
local function checked(issue, method, suffix, body)
    return rocket.http{method=method,path=rest.."/issue/"..rocket.url_encode(issue.key)..(suffix or ""),body=body,
        check={path=rest.."/issue/"..rocket.url_encode(issue.key).."?fields=updated",pointer="/fields/updated",expected=issue.updated}}
end
local function comment_on(key, body)
    return rocket.http{method="POST",path=rest.."/issue/"..rocket.url_encode(key).."/comment",body={body=document(body)}}
end
local function tone(status)
    local category = ((status or {}).statusCategory or {}).key
    if category == "done" then return "success" end
    if category == "indeterminate" then return "accent" end
    return "neutral"
end
local function scope_query(scope, project)
    local inside = 'project = "'..project..'"'
    if scope == "Project" then return inside.." ORDER BY updated DESC" end
    if scope == "This week" then return inside.." AND updated >= -7d ORDER BY updated DESC" end
    return inside.." AND assignee = currentUser() AND resolution = Unresolved ORDER BY updated DESC"
end
-- JQL has an operator or a clause; anything else is words to look for.
local function search_query(typed, project)
    local lower = typed:lower()
    if typed:find("[=~<>]") or lower:find("%f[%a]order%s+by%f[%A]") or lower:find("%f[%a]in%s*%(") then return typed end
    return 'project = "'..project..'" AND text ~ "'..(typed:gsub('[\\"]',"\\%0"))..'" ORDER BY updated DESC'
end
local function load_issue(key)
    local base=rest.."/issue/"..rocket.url_encode(key)
    local detail=get(base.."?fields=summary,description,status,updated,assignee,issuetype")
    local transitions=get(base.."/transitions?expand=transitions.fields")
    local comments=get(base.."/comment?maxResults=5&orderBy=-created")
    local f=detail.fields or {}
    local issue={key=key,summary=text(f.summary,255),description=plain(f.description,0),status=text((f.status or {}).name,100),tone=tone(f.status),
        kind=text((f.issuetype or {}).name,60),assignee=text((f.assignee or {}).displayName,100),transitions={},blocked_transitions=0,comments={}}
    if type(f.updated) == "string" and #f.updated > 0 and #f.updated <= 256 then issue.updated=f.updated end
    local names={}
    for i,t in ipairs(transitions.transitions or {}) do
        if i > 16 then issue.more_transitions=true;break end
        local required=false
        for _,field in pairs(t.fields or {}) do if field.required == true then required=true end end
        local id=text(t.id,32)
        if t.isAvailable ~= false and id:match("^%d+$") then
            if required then issue.blocked_transitions=issue.blocked_transitions+1
            else
                local label=text(t.name,60)
                if names[label] then label=label.." · "..id end
                names[label]=true
                issue.transitions[#issue.transitions+1]={id=id,label=label}
            end
        end
    end
    issue.comment_total=comments.total or #(comments.comments or {})
    for i,c in ipairs(comments.comments or {}) do
        if i > 5 then break end
        issue.comments[i]={author=text((c.author or {}).displayName,100),created=when(c.created),body=text(plain(c.body,0),300)}
    end
    return issue
end
-- A comment belongs to its issue, and a sent one leaves the box.
local function comment_id(s, key)
    return "comment_body_"..key..(s.sent and "_"..s.sent or "")
end
-- Jira may return fewer rows than requested. Follow its offset/total, not our
-- requested page size. Four pages leave room for the panel's other reads under
-- Rocket's eight-read limit; incomplete metadata must never authorize a create.
local function metadata(path, member)
    local rows,offset={},0
    for page=1,4 do
        local meta=get(path.."?maxResults=50&startAt="..offset)
        local items=meta[member]
        if type(items) ~= "table" or meta.startAt ~= offset then return rows,false end
        local count=#items
        if count > 50 then return rows,false end
        for _,item in ipairs(items) do rows[#rows+1]=item end
        offset=offset+count
        if meta.total ~= nil then
            if type(meta.total) ~= "number" or meta.total < offset or meta.total%1 ~= 0 then return rows,false end
            if offset == meta.total and meta.isLast ~= false then return rows,true end
        elseif meta.isLast == true then return rows,true
        else return rows,false end
        if count == 0 or meta.isLast == true then return rows,false end
    end
    return rows,false
end
local function issue_types(project)
    local rows,complete=metadata(rest.."/issue/createmeta/"..rocket.url_encode(project).."/issuetypes","issueTypes")
    local types={}
    for _,t in ipairs(rows) do
        local id=text(tostring(t.id or ""),32)
        if id:match("^%d+$") and t.subtask ~= true then
            if #types >= 32 then complete=false;break end
            types[#types+1]={id=id,name=text(t.name,60)}
        end
    end
    return types,not complete
end
-- Required fields this form has no control for.
local function unfilled(project, type_id)
    local fields,complete=metadata(rest.."/issue/createmeta/"..rocket.url_encode(project).."/issuetypes/"..type_id,"fields")
    local names,description_required={},false
    for _,field in ipairs(fields) do
        local id=field.fieldId or field.key
        if field.required and not field.hasDefaultValue then
            if id == "description" then description_required=true
            elseif id ~= "project" and id ~= "issuetype" and id ~= "summary" then
                if #names < 8 then names[#names+1]=text(field.name or id,80)
                elseif #names == 8 then names[#names+1]="other required fields" end
            end
        end
    end
    return names,complete,description_required
end
local function create_form(s, project, warning)
    local names={};for _,t in ipairs(s.types or {}) do names[#names+1]=t.name end
    local controls={}
    if #names > 0 then controls[#controls+1]=ui.select{id="issue_type",label="Type",options=names,value=names[1]}
    else controls[#controls+1]=ui.text{text="Jira lists no issue types for "..project..".",tone="warning"} end
    if s.more_types then controls[#controls+1]=ui.text{text="Only some issue types are shown. Use Jira if yours is missing.",tone="warning"} end
    controls[#controls+1]=ui.input{id="summary",label="Summary",value=""}
    controls[#controls+1]=ui.multiline{id="description",label="Description",value=""}
    if warning ~= "" then controls[#controls+1]=ui.text{text=warning,tone="warning"} end
    controls[#controls+1]=ui.row{children={ui.button{id="create",label="Review create request",primary=true,disabled=#names==0},ui.button{id="pane_cancel",label="Cancel"}}}
    return ui.card{title="New issue · "..project,children=controls}
end
local function edit_form(issue, warning)
    local controls={
        ui.input{id="edit_summary_"..issue.key,label="Summary",value=issue.summary,disabled=not issue.summary_editable},
        ui.checkbox{id="replace_description_"..issue.key,label="Replace the description",value=false,disabled=not issue.description_editable},
        ui.multiline{id="edit_description_"..issue.key,label="Description",value="",disabled=not issue.description_editable},
        ui.text{text="Rich text stays unless you replace it.",small=true}
    }
    if warning ~= "" then controls[#controls+1]=ui.text{text=warning,tone="warning"} end
    controls[#controls+1]=ui.row{children={ui.button{id="edit_issue",label="Review edits",primary=true,disabled=not issue.updated},ui.button{id="edit_reload",label="Reload"},ui.button{id="pane_cancel",label="Cancel"}}}
    return ui.card{title="Edit "..issue.key,children=controls}
end
local function issue_card(s, issue, warning)
    local meta={}
    if issue.kind ~= "" then meta[#meta+1]=issue.kind end
    if issue.assignee ~= "" then meta[#meta+1]=issue.assignee end
    if issue.updated then meta[#meta+1]="Updated "..when(issue.updated) end
    local controls={
        ui.heading(issue.summary),
        ui.row{children={ui.badge{text=issue.status,tone=issue.tone},ui.text{text=table.concat(meta," · "),small=true}}}
    }
    if warning ~= "" then controls[#controls+1]=ui.text{text=warning,tone="warning"} end
    if issue.description ~= "" then controls[#controls+1]=ui.text(issue.description) end
    local actions={}
    if #issue.transitions > 0 then
        local options={};for _,t in ipairs(issue.transitions) do options[#options+1]=t.label end
        actions[#actions+1]=ui.select{id="transition_"..issue.key,label="Move to",options=options,value=options[1]}
        actions[#actions+1]=ui.button{id="transition_issue",label="Review move",disabled=not issue.updated}
    end
    actions[#actions+1]=ui.button{id="edit_open",label="Edit"}
    controls[#controls+1]=ui.row{children=actions}
    if issue.more_transitions then controls[#controls+1]=ui.text{text="Only the first 16 moves are shown. Use Jira for the rest.",small=true} end
    if issue.blocked_transitions > 0 then controls[#controls+1]=ui.text{text=tostring(issue.blocked_transitions).." moves need extra fields. Use Jira for those.",small=true} end
    controls[#controls+1]=ui.separator{}
    controls[#controls+1]=ui.heading("Comments · "..tostring(issue.comment_total))
    for _,c in ipairs(issue.comments) do
        controls[#controls+1]=ui.column{small=true,children={
            ui.text{text=c.author.." · "..c.created,small=true},
            ui.text(c.body)
        }}
    end
    controls[#controls+1]=ui.separator{}
    controls[#controls+1]=ui.multiline{id=comment_id(s,issue.key),label="Add a comment",value=""}
    controls[#controls+1]=ui.button{id="comment",label="Review comment"}
    return ui.card{title=issue.key,children=controls}
end
return function(ctx)
    local s, values = ctx.state or {}, ctx.values or {}
    local project = text(ctx.settings.project,60)
    if not project:match("^[A-Z][A-Z0-9_]*$") then
        return ui.page{title="Configure Jira",children={ui.text("Set a valid project key in this connection's settings, then refresh.")}}
    end
    -- A pasted cloud ID may carry spaces or capitals.
    local cloud = text(ctx.settings.cloud,64):match("^%s*(.-)%s*$"):lower()
    if cloud ~= "" and not cloud:match("^%x%x%x%x%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%-%x%x%x%x%x%x%x%x%x%x%x%x$") then
        return ui.page{title="Configure Jira",children={ui.text("Check the Cloud ID in this connection's settings, or clear it to use a classic token, then refresh.")}}
    end
    rest = cloud == "" and "/rest/api/3" or "/ex/jira/"..cloud.."/rest/api/3"
    local warning, issue, event = "", s.issue, ctx.event
    -- The request went through, so its form is done.
    if ctx.confirmed then s.pane=nil;s.sent=(s.sent or 0)+1 end
    local target, branch = ctx.target or {}, (ctx.repository or {}).branch
    local short = target.commit and target.commit:sub(1,7)
    -- Writes end the run at Rocket's review.
    if ctx.command == "comment-commit" then
        if issue then comment_on(issue.key,"Commit "..short) else warning="Open an issue first." end
    elseif event == "create" then
        local summary = text(values.summary,256)
        local chosen
        for _,t in ipairs(s.types or {}) do if t.name == values.issue_type then chosen=t end end
        chosen = chosen or (s.types or {})[1]
        if summary == "" or (utf8.len(summary) or 256) > 255 then warning="Enter a summary of 1–255 characters."
        elseif not chosen then warning="Choose an issue type."
        else
            local missing,complete,description_required=unfilled(project,chosen.id)
            if not complete then warning="Jira's required fields could not be fully checked. Create this issue in Jira."
            elseif #missing > 0 then warning=chosen.name.." needs "..table.concat(missing,", ")..", which this form can’t fill. Create it in Jira."
            elseif description_required and not (values.description or ""):find("%S") then warning="This issue type requires a description."
            else
                local fields={project={key=project},summary=summary,issuetype={id=chosen.id}}
                if values.description and #values.description > 0 then fields.description=document(values.description) end
                rocket.http{method="POST",path=rest.."/issue",body={fields=fields}}
            end
        end
    elseif event == "comment" and issue then
        local body = values[comment_id(s,issue.key)] or ""
        if body == "" then warning="Enter a comment." else comment_on(issue.key,body) end
    elseif event == "edit_issue" and issue then
        local summary = values["edit_summary_"..issue.key] or issue.summary
        local fields={}
        if issue.summary_editable and summary ~= issue.summary then fields.summary=summary end
        if values["replace_description_"..issue.key] == true and issue.description_editable then
            fields.description=document(values["edit_description_"..issue.key] or "")
        end
        if not issue.updated then warning="Editing is unavailable because the service returned no update marker."
        elseif summary == "" or (utf8.len(summary) or 256) > 255 then warning="Use a summary of 1–255 characters."
        elseif next(fields) == nil then warning="Change the summary, or choose to replace the description."
        else checked(issue,"PUT","",{fields=fields}) end
    elseif event == "transition_issue" and issue then
        local selected = values["transition_"..issue.key] or (issue.transitions[1] or {}).label
        local transition
        for _,t in ipairs(issue.transitions or {}) do if t.label == selected then transition=t end end
        if not issue.updated or not transition then warning="Load an available move first."
        else checked(issue,"POST","/transitions",{transition={id=transition.id}}) end
    end
    -- What the list shows.
    local scopes = {"My issues","Project","This week"}
    if ctx.repository then scopes[#scopes+1]="This branch" end
    local function show_scope(scope)
        s.scope=scope;s.title=nil;s.page=1;s.tokens={""};s.next_token=nil
        if scope == "This branch" then
            local key=branch_key(branch)
            s.query=key and "key = "..key or nil
            if not key then warning=(branch or "This checkout").." has no issue key in its name." end
        else s.query=scope_query(scope,project) end
    end
    local function show_search(query, title, shown)
        s.scope="";s.query=query;s.title=title;s.page=1;s.tokens={""};s.next_token=nil
        s.search=(s.search or 0)+1;s.search_value=shown
    end
    local function open(key)
        issue=load_issue(key);s.issue=issue;s.pane=nil
    end
    local function edit()
        local meta=get(rest.."/issue/"..rocket.url_encode(issue.key).."/editmeta")
        issue.summary_editable=can_set(meta,"summary");issue.description_editable=can_set(meta,"description")
        s.issue=issue;s.pane="edit"
    end
    if not s.tokens then show_scope("My issues") end
    local search_id="search_"..tostring(s.search or 0)
    if event == "search" then
        local typed=text(values[search_id],1024)
        local key=issue_key(typed)
        if key then open(key)
        elseif typed == "" then show_scope("My issues")
        else
            s.scope="";s.query=search_query(typed,project);s.title=nil;s.page=1;s.tokens={""};s.next_token=nil
        end
    elseif event == "scope" then show_scope(values.scope or "My issues")
    elseif event == "issues" and ctx.row then
        local key=issue_key(ctx.row)
        if key then open(key) end
    elseif event == "next_page" and s.next_token and s.page < 20 then
        s.page=s.page+1;s.tokens[s.page]=s.next_token
    elseif event == "previous_page" and s.page > 1 then s.page=s.page-1
    elseif event == "pane_cancel" then s.pane=nil
    elseif (event == "edit_open" or event == "edit_reload") and issue then
        -- From the issue as it is now, so its update marker is current.
        open(issue.key);edit()
    elseif event == "create_open" or ctx.command == "create-issue" then
        s.types,s.more_types=issue_types(project);s.pane="create"
    elseif event == "open" then
        -- Refresh keeps an open form and what was typed in it.
        local pane=s.pane
        if issue then open(issue.key) end
        if pane == "edit" and issue then edit() else s.pane=pane end
        if s.scope == "This branch" then show_scope(s.scope) end
    elseif ctx.command == "find-commit" then
        show_search('text ~ "'..short..'"',"mention "..short,'text ~ "'..short..'"')
    elseif ctx.command == "open-branch" then
        local key=branch_key(target.branch)
        if key then open(key)
        else issue=nil;s.issue=nil;s.pane=nil;warning=text(target.branch,200).." has no issue key in its name." end
    end
    search_id="search_"..tostring(s.search or 0)
    local rows,keys={},{}
    if s.query then
        local token=(s.tokens or {})[s.page] or ""
        local query=rest.."/search/jql?maxResults=25&fields=summary,status&jql="..rocket.url_encode(s.query)
        if token ~= "" then query=query.."&nextPageToken="..rocket.url_encode(token) end
        local response=get(query)
        s.next_token=type(response.nextPageToken) == "string" and #response.nextPageToken > 0 and #response.nextPageToken <= 512 and response.isLast ~= true and response.nextPageToken or nil
        if type(response.nextPageToken) == "string" and #response.nextPageToken > 512 then warning="The next-page token is too long. Refine the search." end
        for i,item in ipairs(response.issues or {}) do
            if i > 25 then break end
            local f=item.fields or {}
            local key=text(item.key,50)
            if key ~= "" then
                keys[#keys+1]=key
                rows[#rows+1]={{text=key,mono=true},text(f.summary,255),{text=text((f.status or {}).name,80),tone=tone(f.status)}}
            end
        end
    end
    local list={ui.segments{id="scope",options=scopes,value=s.scope}}
    if #rows > 0 then
        list[#list+1]=ui.table{id="issues",columns={"Key","Summary","Status"},keys=keys,rows=rows,selected=issue and issue.key or nil}
    else list[#list+1]=ui.text("No issues match.") end
    if s.page > 1 or s.next_token then
        list[#list+1]=ui.row{children={ui.button{id="previous_page",label="Previous",disabled=s.page<=1},ui.button{id="next_page",label="Next",disabled=not s.next_token or s.page>=20},ui.text{text="Page "..s.page,small=true}}}
    end
    if s.page >= 20 and s.next_token then list[#list+1]=ui.text{text="Refine the search to go beyond 20 pages.",small=true} end
    local right
    if s.pane == "create" then right=create_form(s,project,warning)
    elseif s.pane == "edit" and issue then right=edit_form(issue,warning)
    elseif issue then right=issue_card(s,issue,warning)
    else
        local controls={ui.text("Choose an issue from the list, or search for its key.")}
        if warning ~= "" then table.insert(controls,1,ui.text{text=warning,tone="warning"}) end
        right=ui.card{title="Issue",children=controls}
    end
    return {state=s,view=ui.page{children={
        ui.row{children={
            ui.input{id=search_id,value=s.search_value or "",placeholder="Key, words or JQL",submit="search"},
            ui.button{id="search",label="Search"},
            ui.button{id="create_open",label="Create issue",primary=true}
        }},
        ui.split{focus=(issue or s.pane) and 2 or 1,children={ui.card{title="Issues · "..(s.title or project),children=list},right}}
    }}}
end
