function compose_message(arguments, context)
  return context.view.open("composer", {
    state = { ["draft.recipient"] = arguments.recipient or "" }
  })
end

function send_message(arguments, context)
  return context.notification.show("Message sent")
end

function open_message(arguments, context)
  return context.notification.show("Opening message " .. (arguments.message_id or ""))
end

function discard_draft(arguments, context)
  return context.view.close("composer")
end
