use std::collections::{BTreeMap, HashMap};

use gpui_kit::*;

use crate::{
    CloseTab, CompleteFilter, DeleteLine, DuplicateLine, FormatQuery, MoveLineDown, MoveLineUp,
    MoveTabLeft, MoveTabRight, NewTab, NextTab, PreviousTab, QuickOpen, RefreshTable, RunQuery,
    ToggleAi, ToggleComment, ToggleSidebar,
};

pub(crate) struct ShortcutSpec {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) default_keys: &'static str,
    contexts: &'static [&'static str],
}

pub(crate) const SHORTCUTS: &[ShortcutSpec] = &[
    ShortcutSpec {
        id: "quick_open",
        label: "Quick open table",
        default_keys: "secondary-p",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "new_tab",
        label: "New tab",
        default_keys: "secondary-t",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "close_tab",
        label: "Close tab",
        default_keys: "secondary-w",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "next_tab",
        label: "Next tab",
        default_keys: "ctrl-tab",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "previous_tab",
        label: "Previous tab",
        default_keys: "ctrl-shift-tab",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "move_tab_left",
        label: "Move tab left",
        default_keys: "secondary-shift-pageup",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "move_tab_right",
        label: "Move tab right",
        default_keys: "secondary-shift-pagedown",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "refresh_table",
        label: "Refresh table",
        default_keys: "f5",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "toggle_sidebar",
        label: "Toggle sidebar",
        default_keys: "secondary-b",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "toggle_ai",
        label: "Toggle AI panel",
        default_keys: "secondary-alt-b",
        contexts: &["Koyo"],
    },
    ShortcutSpec {
        id: "run_query",
        label: "Run query",
        default_keys: "secondary-enter",
        contexts: &["Input", "Koyo"],
    },
    ShortcutSpec {
        id: "format_query",
        label: "Format SQL",
        default_keys: "secondary-shift-f",
        contexts: &["Input", "Koyo"],
    },
    ShortcutSpec {
        id: "toggle_comment",
        label: "Toggle SQL comment",
        default_keys: "secondary-/",
        contexts: &["Input && mode == editor", "Koyo"],
    },
    ShortcutSpec {
        id: "move_line_up",
        label: "Move line up",
        default_keys: "alt-up",
        contexts: &["Input && mode == editor"],
    },
    ShortcutSpec {
        id: "move_line_down",
        label: "Move line down",
        default_keys: "alt-down",
        contexts: &["Input && mode == editor"],
    },
    ShortcutSpec {
        id: "duplicate_line",
        label: "Duplicate line",
        default_keys: "shift-alt-down",
        contexts: &["Input && mode == editor"],
    },
    ShortcutSpec {
        id: "delete_line",
        label: "Delete line",
        default_keys: "secondary-shift-k",
        contexts: &["Input && mode == editor"],
    },
    ShortcutSpec {
        id: "complete_filter",
        label: "Complete table filter",
        default_keys: "tab",
        contexts: &["TableFilter"],
    },
];

fn action_for(id: &str) -> Option<Box<dyn Action>> {
    Some(match id {
        "quick_open" => Box::new(QuickOpen),
        "new_tab" => Box::new(NewTab),
        "close_tab" => Box::new(CloseTab),
        "next_tab" => Box::new(NextTab),
        "previous_tab" => Box::new(PreviousTab),
        "move_tab_left" => Box::new(MoveTabLeft),
        "move_tab_right" => Box::new(MoveTabRight),
        "refresh_table" => Box::new(RefreshTable),
        "toggle_sidebar" => Box::new(ToggleSidebar),
        "toggle_ai" => Box::new(ToggleAi),
        "run_query" => Box::new(RunQuery),
        "format_query" => Box::new(FormatQuery),
        "toggle_comment" => Box::new(ToggleComment),
        "move_line_up" => Box::new(MoveLineUp),
        "move_line_down" => Box::new(MoveLineDown),
        "duplicate_line" => Box::new(DuplicateLine),
        "delete_line" => Box::new(DeleteLine),
        "complete_filter" => Box::new(CompleteFilter),
        _ => return None,
    })
}

pub(crate) fn keys_for<'a>(
    shortcuts: &'a BTreeMap<String, String>,
    spec: &ShortcutSpec,
) -> &'a str {
    shortcuts
        .get(spec.id)
        .map(String::as_str)
        .unwrap_or(spec.default_keys)
}

pub(crate) fn display_keys(keys: &str) -> String {
    keys.split('-')
        .map(|part| match part {
            "secondary" => "Ctrl/Cmd",
            "ctrl" => "Ctrl",
            "cmd" => "Cmd",
            "alt" => "Alt",
            "shift" => "Shift",
            "enter" => "Enter",
            "escape" => "Esc",
            "space" => "Space",
            "tab" => "Tab",
            "backspace" => "Backspace",
            "delete" => "Delete",
            "home" => "Home",
            "end" => "End",
            "up" => "Up",
            "down" => "Down",
            "left" => "Left",
            "right" => "Right",
            "pageup" => "PageUp",
            "pagedown" => "PageDown",
            other => other,
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

fn canonical_keys(keys: &str, os: OperatingSystem) -> Option<String> {
    Keystroke::parse_with_os(keys, os)
        .ok()
        .map(|keystroke| keystroke.unparse(os))
}

/// Return the label of the action already using this key combination. The
/// comparison resolves `secondary-` for the current OS, so ctrl/cmd aliases
/// cannot slip past the conflict check.
pub(crate) fn shortcut_conflict(
    shortcuts: &BTreeMap<String, String>,
    action: &str,
    keys: &str,
    os: OperatingSystem,
) -> Option<&'static str> {
    let candidate = canonical_keys(keys, os)?;
    SHORTCUTS.iter().find_map(|spec| {
        if spec.id == action {
            return None;
        }
        let used = canonical_keys(keys_for(shortcuts, spec), os)?;
        (used == candidate).then_some(spec.label)
    })
}

/// Remove old duplicate overrides imported from a previous state file. Valid
/// customs win over defaults, but no two effective bindings may share a key.
pub(crate) fn sanitize_shortcuts(
    shortcuts: &BTreeMap<String, String>,
    os: OperatingSystem,
) -> BTreeMap<String, String> {
    let mut sanitized = shortcuts.clone();
    sanitized.retain(|id, keys| {
        SHORTCUTS.iter().any(|spec| spec.id == id) && canonical_keys(keys, os).is_some()
    });
    // Defaults are unique. Only a saved custom can introduce a duplicate;
    // remove the custom side of each conflict and repeat until stable.
    loop {
        let mut used = HashMap::new();
        let mut conflict = None;
        for spec in SHORTCUTS {
            let Some(key) = canonical_keys(keys_for(&sanitized, spec), os) else {
                continue;
            };
            if let Some(previous) = used.insert(key, spec.id) {
                conflict = Some((previous, spec.id));
                break;
            }
        }
        let Some((first, second)) = conflict else {
            break;
        };
        if sanitized.remove(first).is_none() && sanitized.remove(second).is_none() {
            break;
        }
    }
    sanitized
}

pub(crate) fn validate_keys(id: &str, keys: &str, cx: &App) -> Result<(), String> {
    let Some(action) = action_for(id) else {
        return Err("Unknown shortcut action.".into());
    };
    let Some(spec) = SHORTCUTS.iter().find(|spec| spec.id == id) else {
        return Err("Unknown shortcut action.".into());
    };
    for context in spec.contexts {
        let predicate = KeyBindingContextPredicate::parse(context)
            .map_err(|error| format!("Invalid shortcut context: {error}"))?;
        KeyBinding::load(
            keys,
            action.boxed_clone(),
            Some(predicate.into()),
            false,
            None,
            cx,
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn bindings_for(shortcuts: &BTreeMap<String, String>, cx: &App) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for spec in SHORTCUTS {
        let keys = keys_for(shortcuts, spec);
        let Some(action) = action_for(spec.id) else {
            continue;
        };
        for context in spec.contexts {
            let Ok(predicate) = KeyBindingContextPredicate::parse(context) else {
                continue;
            };
            if let Ok(binding) = KeyBinding::load(
                keys,
                action.boxed_clone(),
                Some(predicate.into()),
                false,
                None,
                cx,
            ) {
                bindings.push(binding);
            }
        }
    }
    bindings
}

#[cfg(test)]
mod tests {
    use super::{SHORTCUTS, action_for, display_keys, sanitize_shortcuts, shortcut_conflict};
    use crate::{RunQuery, ToggleSidebar};
    use gpui_kit::{Action as _, KeyBindingContextPredicate, Keystroke, OperatingSystem};
    use std::collections::BTreeMap;

    #[test]
    fn action_namespace_matches_the_rebind_filter() {
        assert_eq!(RunQuery.name(), "koyo::RunQuery");
        assert_eq!(ToggleSidebar.name(), "koyo::ToggleSidebar");
    }

    #[test]
    fn shortcut_ids_are_unique_and_actions_exist() {
        let mut ids = std::collections::HashSet::new();
        for spec in SHORTCUTS {
            assert!(ids.insert(spec.id), "duplicate shortcut id {}", spec.id);
            assert!(action_for(spec.id).is_some(), "missing action {}", spec.id);
            assert!(!spec.default_keys.is_empty());
        }
    }

    #[test]
    fn display_uses_secondary_label() {
        assert_eq!(display_keys("secondary-enter"), "Ctrl/Cmd + Enter");
    }

    #[test]
    fn default_shortcuts_are_unique() {
        for os in [OperatingSystem::Linux, OperatingSystem::Mac] {
            let sanitized = sanitize_shortcuts(&BTreeMap::new(), os);
            assert!(sanitized.is_empty());
            let keys = SHORTCUTS
                .iter()
                .map(|spec| {
                    Keystroke::parse_with_os(spec.default_keys, os)
                        .unwrap()
                        .unparse(os)
                })
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(keys.len(), SHORTCUTS.len());
        }
    }

    #[test]
    fn conflicts_are_rejected_and_old_duplicates_are_removed() {
        assert_eq!(
            shortcut_conflict(
                &BTreeMap::new(),
                "run_query",
                "secondary-w",
                OperatingSystem::Linux
            ),
            Some("Close tab")
        );
        assert_eq!(
            shortcut_conflict(
                &BTreeMap::new(),
                "run_query",
                "ctrl-alt-r",
                OperatingSystem::Linux
            ),
            None
        );

        let saved = BTreeMap::from([
            ("run_query".to_string(), "secondary-w".to_string()),
            ("new_tab".to_string(), "ctrl-alt-n".to_string()),
        ]);
        let sanitized = sanitize_shortcuts(&saved, OperatingSystem::Linux);
        assert!(!sanitized.contains_key("run_query"));
        assert_eq!(sanitized["new_tab"], "ctrl-alt-n");
    }

    #[test]
    fn default_shortcuts_and_contexts_parse() {
        for spec in SHORTCUTS {
            for os in [OperatingSystem::Linux, OperatingSystem::Mac] {
                assert!(
                    Keystroke::parse_with_os(spec.default_keys, os).is_ok(),
                    "{} does not parse on {:?}",
                    spec.default_keys,
                    os
                );
            }
            for context in spec.contexts {
                assert!(
                    KeyBindingContextPredicate::parse(context).is_ok(),
                    "invalid context {context}"
                );
            }
        }
    }
}

impl super::KoyoApp {
    pub(crate) fn apply_shortcut_bindings(&mut self, cx: &mut Context<Self>) {
        self.shortcuts = sanitize_shortcuts(&self.shortcuts, cx.platform_info().os);
        let existing = cx
            .key_bindings()
            .borrow()
            .bindings()
            .filter(|binding| !binding.action().name().starts_with("koyo::"))
            .cloned()
            .collect::<Vec<_>>();
        // While recording, remove Koyo actions so pressing an old shortcut
        // cannot run it before the settings key listener sees the event.
        let configured = if self.shortcut_capture.is_some() {
            Vec::new()
        } else {
            bindings_for(&self.shortcuts, cx)
        };
        cx.clear_key_bindings();
        cx.bind_keys(existing);
        cx.bind_keys(configured);
    }
}
