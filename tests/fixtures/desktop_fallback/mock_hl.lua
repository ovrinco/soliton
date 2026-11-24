-- A stand-in for the compositor's `hl` table, enough to run the rendered
-- fallback block outside the compositor: monitors and workspaces as plain
-- tables, `hl.dispatch(hl.dsp.workspace.move(...))` moves a workspace, and
-- `fire(event)` calls the handlers subscribed to that event. An output is
-- `{ name, dpms_status, disabled }`; `set_outputs` changes only the outputs
-- (sleep, wake, unplug, replug) and leaves every workspace where it is.
local monitors, workspaces, handlers = {}, {}, {}
RULES = {}
MOVES = 0

local function by_name(name)
  for _, m in ipairs(monitors) do
    if m.name == name then return m end
  end
end

function set_topology(outputs, placement)
  monitors = {}
  for _, o in ipairs(outputs) do
    monitors[#monitors + 1] = { name = o[1], dpms_status = o[2], disabled = o[3] or false, is_mirror = false, width = 1920, height = 1080 }
  end
  local old = {}
  for _, w in ipairs(workspaces) do old[w.id] = w end
  workspaces = {}
  for _, p in ipairs(placement) do
    local w = old[p[1]] or { id = p[1], special = false }
    w.monitor = by_name(p[2])
    workspaces[#workspaces + 1] = w
  end
end

function set_outputs(outputs)
  local kept = {}
  for _, w in ipairs(workspaces) do kept[#kept + 1] = { w.id, w.monitor and w.monitor.name or "" } end
  set_topology(outputs, kept)
  for _, w in ipairs(workspaces) do
    -- a workspace on an output no longer listed still names it, as the compositor's stranded workspace does
    if not w.monitor then
      for _, k in ipairs(kept) do
        if k[1] == w.id then w.monitor = { name = k[2] } end
      end
    end
  end
end

function placement()
  local rows = {}
  for _, w in ipairs(workspaces) do rows[#rows + 1] = w.id .. "=" .. (w.monitor and w.monitor.name or "") end
  return table.concat(rows, " ")
end

function fire(event)
  for _, h in ipairs(handlers[event] or {}) do h() end
end

hl = {
  get_monitors = function() return monitors end,
  get_workspaces = function() return workspaces end,
  dispatch = function(d) d() end,
  on = function(event, cb)
    handlers[event] = handlers[event] or {}
    local list = handlers[event]
    list[#list + 1] = cb
    return { remove = function()
      for i, h in ipairs(list) do if h == cb then table.remove(list, i) end end
    end }
  end,
  workspace_rule = function(spec)
    local rule = { spec = spec, enabled = true }
    function rule:set_enabled(v) self.enabled = v end
    RULES[#RULES + 1] = rule
    return rule
  end,
  dsp = { workspace = { move = function(spec)
    return function()
      local target = type(spec.monitor) == "table" and spec.monitor or by_name(spec.monitor)
      for _, w in ipairs(workspaces) do
        if w == spec.workspace or tostring(w.id) == spec.workspace then
          print("move " .. w.id .. " " .. target.name)
          MOVES = MOVES + 1
          w.monitor = target
        end
      end
    end
  end } },
}

function enabled_rules()
  local rows = {}
  for _, r in ipairs(RULES) do
    if r.enabled then rows[#rows + 1] = r.spec.workspace .. "@" .. r.spec.monitor end
  end
  return table.concat(rows, " ")
end
