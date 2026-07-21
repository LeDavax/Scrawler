-- Recognised priority labels, keyed by lowercase for case-insensitive matching
-- against whatever an agent passes in (e.g. "high", "High", "HIGH").
local PRIORITY_LABELS = {
  low    = "Low",
  normal = "Normal",
  high   = "High",
  urgent = "Urgent",
}

local function normalize_priority(value, fallback)
  if value == nil then
    return fallback
  end
  return PRIORITY_LABELS[string.lower(value)] or fallback
end

-- `arguments.read_receipt` is a real boolean when provided, so `arguments.read_receipt
-- or fallback` would wrongly replace an explicit `false` with `fallback`. This only
-- falls back when the argument was omitted entirely (nil).
local function default_if_nil(value, fallback)
  if value == nil then
    return fallback
  end
  return value
end

function compose_message(arguments, context)
  return context.view.open("composer", {
    state = {
      ["draft.recipient"]    = arguments.recipient or "",
      ["draft.body"]         = arguments.body or "",
      ["draft.priority"]     = normalize_priority(arguments.priority, "High"),
      ["draft.read_receipt"] = default_if_nil(arguments.read_receipt, false),
    }
  })
end

function send_message(arguments, context)
  -- A field passed to send_message overrides the current draft; otherwise we
  -- fall back to whatever is already in the composer (typed by a human, set
  -- by an earlier compose_message call, or left over from state).
  local recipient = arguments.recipient or context.state.get("draft.recipient") or ""
  local body      = arguments.body or context.state.get("draft.body") or ""
  local priority  = normalize_priority(arguments.priority, context.state.get("draft.priority") or "High")
  local read_receipt = default_if_nil(
    arguments.read_receipt,
    context.state.get("draft.read_receipt") == "true"
  )

  if recipient == "" then
    return context.notification.show("Add a recipient before sending")
  end
  if body == "" then
    return context.notification.show("Write a message before sending")
  end

  local receipt_note = ""
  if read_receipt then
    receipt_note = " (read receipt requested)"
  end

  return {
    context.notification.show(
      "Message sent to " .. recipient .. " [" .. priority .. " priority]" .. receipt_note
    ),
    -- Reset the draft back to its defaults so the composer is empty next time
    -- it opens, and actually close the dialog instead of leaving it on screen.
    context.state.set("draft.recipient", ""),
    context.state.set("draft.body", ""),
    context.state.set("draft.priority", "High"),
    context.state.set("draft.read_receipt", false),
    context.form.reset("composer"),
    context.view.close("composer"),
  }
end

function open_message(arguments, context)
  return context.notification.show("Opening message " .. (arguments.message_id or ""))
end

function discard_draft(arguments, context)
  return {
    context.state.set("draft.recipient", ""),
    context.state.set("draft.body", ""),
    context.state.set("draft.priority", "High"),
    context.state.set("draft.read_receipt", false),
    context.form.reset("composer"),
    context.view.close("composer"),
  }
end