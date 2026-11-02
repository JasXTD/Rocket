-- Every control an extension can use. Each section shows the live control beside its Lua.
local sections = {"Controls", "Text", "Data", "Layout", "Commands"}

local function times(count)
    if count == 1 then return "once" end
    return tostring(count) .. " times"
end

local function pane(demo, lua)
    return ui.split { children = {
        ui.card { title = "Try it", children = demo },
        ui.card { title = "The Lua", children = { ui.code { text = lua } } },
    } }
end

local function controls(name, count)
    return pane({
        ui.input { id = "name", label = "Name", value = "Rocket" },
        ui.select { id = "layout", label = "Theme example", options = {"Navy", "Neutral", "Accent"}, value = "Navy" },
        ui.checkbox { id = "details", label = "Include details", value = true },
        ui.row { children = {
            ui.button { id = "increment", label = "Update panel", primary = true },
            ui.button { id = "plain", label = "Plain" },
            ui.button { id = "off", label = "Disabled", disabled = true },
        } },
        ui.text("Hello, " .. name .. " · updated " .. times(count)),
    }, [[
ui.input { id = "name", label = "Name", value = "Rocket" },
ui.select { id = "layout", label = "Theme example",
  options = {"Navy", "Neutral", "Accent"}, value = "Navy" },
ui.checkbox { id = "details", label = "Include details", value = true },
ui.row { children = {
  ui.button { id = "increment", label = "Update panel", primary = true },
  ui.button { id = "plain", label = "Plain" },
  ui.button { id = "off", label = "Disabled", disabled = true },
} },
ui.text("Hello, " .. name .. " · updated " .. times(count)),]])
end

local function text()
    return pane({
        ui.heading("A heading"),
        ui.text("Plain text wraps to the width of its card."),
        ui.text { text = "Text can carry a tone.", tone = "warning" },
        ui.text { text = "Small text for what matters less.", small = true },
        ui.row { children = {
            ui.badge { text = "Neutral" },
            ui.badge { text = "Accent", tone = "accent" },
            ui.badge { text = "Success", tone = "success" },
            ui.badge { text = "Warning", tone = "warning" },
            ui.badge { text = "Error", tone = "error" },
        } },
        ui.code { text = 'return ui.text("Hello")' },
    }, [[
ui.heading("A heading"),
ui.text("Plain text wraps to the width of its card."),
ui.text { text = "Text can carry a tone.",
  tone = "warning" },
ui.text { text = "Small text for what matters less.",
  small = true },
ui.row { children = {
  ui.badge { text = "Neutral" },
  ui.badge { text = "Accent", tone = "accent" },
  ui.badge { text = "Success", tone = "success" },
} },
ui.code { text = 'return ui.text("Hello")' },]])
end

local function data(row, count)
    local ready = { text = "Ready", tone = "success" }
    return pane({
        ui.table {
            id = "rows", selected = row,
            columns = {"Feature", "Example", "State"},
            keys = {"unicode", "marks", "state"},
            rows = {
                {"Unicode", "資料 · 日本語 · 한국어 · 🚀", ready},
                {"Combining marks", "café · é", ready},
                {"State", tostring(count), { text = "Live", tone = "accent" }},
            },
        },
        ui.progress { value = 0.65, label = "Example progress" },
    }, [[
local ready = { text = "Ready", tone = "success" }
ui.table {
  id = "rows", selected = ctx.row,
  columns = {"Feature", "Example", "State"},
  keys = {"unicode", "marks", "state"},
  rows = {
    {"Unicode", "資料 · 日本語 · 한국어", ready},
    {"Combining marks", "café · é", ready},
    {"State", tostring(count),
      { text = "Live", tone = "accent" }},
  },
},
ui.progress { value = 0.65, label = "Example progress" },]])
end

local function layout()
    return pane({
        ui.split { children = {
            ui.card { title = "Left", children = { ui.text("Two panes side by side.") } },
            ui.card { title = "Right", children = {
                ui.row { children = {
                    ui.button { id = "one", label = "One" },
                    ui.button { id = "two", label = "Two" },
                } },
                ui.separator {},
                ui.text("A narrow window shows them as tabs."),
            } },
        } },
    }, [[
ui.split { children = {
  ui.card { title = "Left", children = {
    ui.text("Two panes side by side."),
  } },
  ui.card { title = "Right", children = {
    ui.row { children = {
      ui.button { id = "one", label = "One" },
      ui.button { id = "two", label = "Two" },
    } },
    ui.separator {},
    ui.text("A narrow window shows them as tabs."),
  } },
} },]])
end

local function commands(last)
    local demo = {}
    if last then
        demo[1] = ui.table { columns = {"Field", "Value"}, rows = {
            {{ text = "ctx.event", mono = true }, { text = last.event, mono = true }},
            {{ text = "ctx.command", mono = true }, { text = last.command, mono = true }},
            {{ text = "ctx.target.kind", mono = true }, { text = last.kind, mono = true }},
            {{ text = "ctx.target.commit", mono = true }, { text = last.commit, mono = true }},
        } }
    else
        demo[1] = ui.text("Right-click a commit and choose PlayGround, then Show what a command receives.")
    end
    return pane(demo, [[
if ctx.command == "show-target" then
  return ui.table { columns = {"Field", "Value"}, rows = {
    {"ctx.event", ctx.event},
    {"ctx.command", ctx.command},
    {"ctx.target.kind", ctx.target.kind},
    {"ctx.target.commit", ctx.target.commit},
  } }
end]])
end

return function(ctx)
    local s = ctx.state or {}
    s.count = s.count or 0
    s.section = s.section or sections[1]
    if ctx.event == "increment" then s.count = s.count + 1 end
    if ctx.event == "section" then s.section = ctx.values.section or s.section end
    if ctx.event == "rows" then s.row = ctx.row end
    if ctx.command then
        local target = ctx.target or {}
        s.section = "Commands"
        s.last = {
            event = ctx.event, command = ctx.command,
            kind = target.kind or "none", commit = target.commit or "none",
        }
    end
    local name = ctx.values.name or "Rocket"
    local body
    if s.section == "Text" then body = text()
    elseif s.section == "Data" then body = data(s.row, s.count)
    elseif s.section == "Layout" then body = layout()
    elseif s.section == "Commands" then body = commands(s.last)
    else body = controls(name, s.count) end
    return {
        state = s,
        view = ui.page { children = {
            ui.text("Every control an extension can use, with the Lua that draws it."),
            ui.segments { id = "section", options = sections, value = s.section },
            body,
        } }
    }
end
