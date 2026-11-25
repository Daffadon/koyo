use super::*;

impl KoyoApp {
    pub(crate) fn accept_completion_or_indent(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.focused_index().and_then(|index| {
            self.tabs.get(index).and_then(|tab| match &tab.body {
                TabBody::Query(query) => Some(query.editor.clone()),
                TabBody::Table(_) => None,
            })
        }) else {
            return;
        };
        let (completion_open, handle) = {
            let editor = editor.read(cx);
            (
                editor.completion_menu_state().open,
                editor.focus_handle(cx).clone(),
            )
        };
        if completion_open {
            handle.dispatch_action(
                &Enter {
                    secondary: false,
                    shift: false,
                },
                window,
                cx,
            );
        } else {
            handle.dispatch_action(&IndentInline, window, cx);
        }
        cx.stop_propagation();
    }

    /// Ctrl+Backspace after horizontal whitespace removes that whitespace
    /// without also eating the word before it. Otherwise the editor's normal
    /// delete-previous-word behavior applies.
    pub(crate) fn handle_ctrl_backspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(editor) = self.focused_query_editor() else {
            return false;
        };
        let (value, selection, handle) = {
            let editor = editor.read(cx);
            (
                editor.value().to_string(),
                editor.selected_range(),
                editor.focus_handle(cx).clone(),
            )
        };
        if selection.is_empty()
            && let Some(start) = previous_horizontal_whitespace_start(&value, selection.end)
        {
            editor.update(cx, |editor, cx| {
                editor.set_selected_range(start..selection.end, cx);
                editor.replace("", window, cx);
            });
            return true;
        }
        handle.dispatch_action(&DeleteToPreviousWordStart, window, cx);
        true
    }
}
