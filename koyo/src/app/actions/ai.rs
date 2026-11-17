use std::time::{Duration, Instant};

use gpui_kit::Window;
use gpui_kit::prelude::*;
use tokio::sync::mpsc;

use crate::app::*;

pub(crate) fn restore_ai_threads(saved: &PersistedState) -> (Vec<AiThread>, u64, u64) {
    let mut threads = if !saved.ai_threads.is_empty() {
        saved
            .ai_threads
            .iter()
            .map(|thread| AiThread {
                id: thread.id,
                title: if thread.title.trim().is_empty() {
                    thread_title_from_messages(&thread.messages)
                } else {
                    thread.title.clone()
                },
                messages: thread.messages.clone(),
            })
            .collect::<Vec<_>>()
    } else if !saved.ai_messages.is_empty() {
        let messages = saved.ai_messages.clone();
        vec![AiThread {
            id: 1,
            title: thread_title_from_messages(&messages),
            messages,
        }]
    } else {
        Vec::new()
    };
    if threads.is_empty() {
        threads.push(AiThread::new(1));
    }
    let active = if threads
        .iter()
        .any(|thread| thread.id == saved.ai_active_thread)
    {
        saved.ai_active_thread
    } else {
        threads[0].id
    };
    let max_id = threads
        .iter()
        .map(|thread| thread.id)
        .max()
        .unwrap_or_default();
    let next = saved.next_ai_thread_id.max(max_id.saturating_add(1)).max(1);
    (threads, active, next)
}

impl KoyoApp {
    pub(crate) fn ai_messages(&self) -> &[AiMessage] {
        self.ai_threads
            .iter()
            .find(|thread| thread.id == self.ai_active_thread)
            .map(|thread| thread.messages.as_slice())
            .unwrap_or(&[])
    }

    fn active_ai_thread_mut(&mut self) -> Option<&mut AiThread> {
        self.ai_threads
            .iter_mut()
            .find(|thread| thread.id == self.ai_active_thread)
    }

    pub(crate) fn toggle_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_open = !self.ai_open;
        if !self.ai_open {
            self.ai_settings_open = false;
            // Closing removes the focused composer. Put focus back on the
            // workspace so the global shortcut can reopen the panel.
            self.root_focus.focus(window, cx);
        } else if !self.ai_settings_open {
            self.focus_ai_input(window, cx);
        }
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn close_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_open = false;
        self.ai_settings_open = false;
        self.root_focus.focus(window, cx);
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn set_ai_width(&mut self, width: f32, cx: &mut Context<Self>) {
        let width = width.clamp(MIN_AI_WIDTH, MAX_AI_WIDTH);
        if (self.ai_width - width).abs() > 0.5 {
            self.ai_width = width;
            self.ai_resizing = true;
            cx.notify();
        }
    }

    pub(crate) fn open_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.ai_settings_open {
            self.ai_settings_open = true;
            self.load_ai_config_input(window, cx);
        }
        self.ai_config_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(crate) fn cancel_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_settings_open = false;
        self.load_ai_config_input(window, cx);
        self.ai_config_error = AiConfig::parse(&self.ai_config_json)
            .err()
            .unwrap_or_default();
        self.focus_ai_input(window, cx);
        cx.notify();
    }

    pub(crate) fn save_ai_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let json = self.ai_config_input.read(cx).value().to_string();
        match AiConfig::parse(&json) {
            Ok(config) => {
                self.ai_config_json = json;
                self.ai_config_error.clear();
                self.ai_model = config.model;
                self.ai_settings_open = false;
                self.persist_state(cx);
                self.focus_ai_input(window, cx);
            }
            Err(error) => self.ai_config_error = error,
        }
        cx.notify();
    }

    pub(crate) fn new_ai_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_pending {
            return;
        }
        if self.ai_messages().is_empty() {
            self.focus_ai_input(window, cx);
            return;
        }
        if self.ai_threads.len() >= AI_THREAD_LIMIT {
            self.status =
                format!("Chat thread limit ({AI_THREAD_LIMIT}) reached. Delete a thread first.");
            cx.notify();
            return;
        }
        let id = self.next_ai_thread_id;
        self.next_ai_thread_id = self.next_ai_thread_id.wrapping_add(1).max(1);
        self.ai_threads.push(AiThread::new(id));
        self.ai_active_thread = id;
        self.ai_error.clear();
        self.ai_settings_open = false;
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        self.focus_ai_input(window, cx);
        cx.notify();
    }

    pub(crate) fn switch_ai_thread(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.ai_pending || self.ai_threads.len() < 2 {
            return;
        }
        let Some(index) = self
            .ai_threads
            .iter()
            .position(|thread| thread.id == self.ai_active_thread)
        else {
            return;
        };
        let next = (index as isize + delta).rem_euclid(self.ai_threads.len() as isize) as usize;
        if next == index {
            return;
        }
        self.ai_active_thread = self.ai_threads[next].id;
        self.ai_error.clear();
        self.ai_settings_open = false;
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn request_delete_ai_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_pending || self.ai_threads.len() <= 1 {
            return;
        }
        self.confirm(
            window,
            cx,
            "Delete chat thread?",
            "This permanently removes this thread and its messages.",
            |this, cx| this.delete_ai_thread(cx),
        );
    }

    fn delete_ai_thread(&mut self, cx: &mut Context<Self>) {
        if self.ai_pending || self.ai_threads.len() <= 1 {
            return;
        }
        let Some(index) = self
            .ai_threads
            .iter()
            .position(|thread| thread.id == self.ai_active_thread)
        else {
            return;
        };
        self.ai_threads.remove(index);
        let next = index.min(self.ai_threads.len() - 1);
        self.ai_active_thread = self.ai_threads[next].id;
        self.ai_error.clear();
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn compact_ai_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_pending || self.ai_messages().is_empty() {
            return;
        }
        let config = match AiConfig::parse(&self.ai_config_json) {
            Ok(config) => config,
            Err(error) => {
                self.ai_config_error = error.clone();
                self.ai_error = error;
                self.ai_settings_open = true;
                self.load_ai_config_input(window, cx);
                cx.notify();
                return;
            }
        };

        self.ai_generation = self.ai_generation.wrapping_add(1);
        let generation = self.ai_generation;
        self.ai_pending = true;
        self.ai_compacting = true;
        self.ai_error.clear();
        self.ai_config_error.clear();
        cx.notify();

        let messages = transcript_snapshot(self.ai_messages())
            .into_iter()
            .filter(|message| !message.content.trim().is_empty())
            .collect::<Vec<_>>();
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        let task = self.runtime.spawn(async move {
            let _ = result_tx.send(ai::compact(&config, &messages).await);
        });
        self.ai_task = Some(task);
        cx.spawn(async move |this, cx| {
            let result = result_rx
                .await
                .unwrap_or_else(|_| Err("AI compaction was stopped.".to_string()));
            let _ = this.update(cx, |this, cx| {
                if this.ai_generation == generation {
                    this.finish_ai_compact(result, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn stop_ai(&mut self, cx: &mut Context<Self>) {
        if !self.ai_pending {
            return;
        }
        self.ai_generation = self.ai_generation.wrapping_add(1);
        if let Some(task) = self.ai_task.take() {
            task.abort();
        }
        let compacting = self.ai_compacting;
        self.ai_pending = false;
        self.ai_compacting = false;
        if !compacting && let Some(thread) = self.active_ai_thread_mut() {
            discard_empty_assistant(&mut thread.messages);
        }
        self.ai_error.clear();
        self.status = "AI response stopped.".into();
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn send_ai_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ai_pending {
            return;
        }
        let prompt = self.ai_input.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        let config = match AiConfig::parse(&self.ai_config_json) {
            Ok(config) => config,
            Err(error) => {
                self.ai_config_error = error.clone();
                self.ai_error = error;
                self.ai_settings_open = true;
                self.load_ai_config_input(window, cx);
                cx.notify();
                return;
            }
        };

        self.ai_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        if let Some(thread) = self.active_ai_thread_mut() {
            if thread.messages.is_empty() {
                thread.title = thread_title(&prompt);
            }
            thread.messages.push(AiMessage::user(prompt));
            thread.messages.push(AiMessage::assistant(""));
        }
        self.ai_generation = self.ai_generation.wrapping_add(1);
        let generation = self.ai_generation;
        self.ai_pending = true;
        self.ai_compacting = false;
        self.ai_config_error.clear();
        self.ai_error.clear();
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();

        let messages = transcript_snapshot(self.ai_messages())
            .into_iter()
            .filter(|message| !message.content.trim().is_empty())
            .collect::<Vec<_>>();
        let (sender, mut receiver) = mpsc::channel(32);
        let task = self
            .runtime
            .spawn(async move { ai::stream(&config, &messages, sender).await });
        self.ai_task = Some(task);
        cx.spawn(async move |this, cx| {
            let mut terminal = false;
            let mut pending_text = String::new();
            let mut last_draw = Instant::now();
            const DRAW_INTERVAL: Duration = Duration::from_millis(50);

            while let Some(event) = receiver.recv().await {
                let event = if let AiStreamEvent::Delta(text) = event {
                    pending_text.push_str(&text);
                    // Coalesce every delta already queued so a burst of SSE
                    // events does not reparse the Markdown transcript repeatedly.
                    let mut terminal_event = None;
                    while let Ok(next) = receiver.try_recv() {
                        match next {
                            AiStreamEvent::Delta(more) => pending_text.push_str(&more),
                            other => {
                                terminal_event = Some(other);
                                break;
                            }
                        }
                    }
                    if terminal_event.is_none() && last_draw.elapsed() < DRAW_INTERVAL {
                        continue;
                    }
                    let text = std::mem::take(&mut pending_text);
                    last_draw = Instant::now();
                    let _ = this.update(cx, |this, cx| {
                        if this.ai_generation == generation {
                            this.append_ai_delta(text, cx);
                        }
                    });
                    match terminal_event {
                        Some(event) => event,
                        None => continue,
                    }
                } else {
                    event
                };
                if !pending_text.is_empty() {
                    let text = std::mem::take(&mut pending_text);
                    let _ = this.update(cx, |this, cx| {
                        if this.ai_generation == generation {
                            this.append_ai_delta(text, cx);
                        }
                    });
                }
                match event {
                    AiStreamEvent::Delta(_) => {}
                    AiStreamEvent::Done => {
                        terminal = true;
                        let _ = this.update(cx, |this, cx| {
                            if this.ai_generation == generation {
                                this.finish_ai_stream(None, cx);
                            }
                        });
                        break;
                    }
                    AiStreamEvent::Error(error) => {
                        terminal = true;
                        let _ = this.update(cx, |this, cx| {
                            if this.ai_generation == generation {
                                this.finish_ai_stream(Some(error), cx);
                            }
                        });
                        break;
                    }
                }
            }
            if !terminal {
                if !pending_text.is_empty() {
                    let _ = this.update(cx, |this, cx| {
                        if this.ai_generation == generation {
                            this.append_ai_delta(pending_text, cx);
                        }
                    });
                }
                let _ = this.update(cx, |this, cx| {
                    if this.ai_generation == generation {
                        this.finish_ai_stream(
                            Some("AI stream ended unexpectedly.".to_string()),
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }

    fn append_ai_delta(&mut self, text: String, cx: &mut Context<Self>) {
        if let Some(thread) = self.active_ai_thread_mut() {
            append_ai_delta(&mut thread.messages, &text);
        }
        self.ai_scroll.scroll_to_bottom();
        cx.notify();
    }

    fn finish_ai_stream(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.ai_task = None;
        self.ai_pending = false;
        self.ai_compacting = false;
        if let Some(thread) = self.active_ai_thread_mut() {
            self.ai_error = finish_reply(&mut thread.messages, error);
        }
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();
    }

    fn finish_ai_compact(&mut self, result: Result<String, String>, cx: &mut Context<Self>) {
        self.ai_task = None;
        self.ai_pending = false;
        self.ai_compacting = false;
        if let Some(thread) = self.active_ai_thread_mut() {
            self.ai_error = apply_compact_result(&mut thread.messages, result);
        }
        if self.ai_error.is_empty() {
            self.status = "AI context compacted.".into();
        }
        self.ai_scroll.scroll_to_bottom();
        self.persist_state(cx);
        cx.notify();
    }

    pub(crate) fn ai_threads_snapshot(&self) -> Vec<AiThread> {
        self.ai_threads
            .iter()
            .map(|thread| AiThread {
                id: thread.id,
                title: thread.title.clone(),
                messages: transcript_snapshot(&thread.messages),
            })
            .collect()
    }

    fn load_ai_config_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let json = self.ai_config_json.clone();
        self.ai_config_input.update(cx, |input, cx| {
            input.set_value(json, window, cx);
        });
    }

    fn focus_ai_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.ai_input.update(cx, |input, cx| {
            input.focus(window, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_migrates_legacy_transcript_into_a_thread() {
        let mut saved = PersistedState::default();
        saved.ai_messages = vec![
            AiMessage::user("How do I list users?"),
            AiMessage::assistant("Run a SELECT."),
        ];

        let (threads, active, next) = restore_ai_threads(&saved);

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, 1);
        assert_eq!(threads[0].title, "How do I list users?");
        assert_eq!(threads[0].messages.len(), 2);
        assert_eq!(active, 1);
        assert_eq!(next, 2);
    }

    #[test]
    fn restore_keeps_full_thread_transcripts() {
        let mut saved = PersistedState::default();
        saved.ai_threads = vec![AiThread {
            id: 1,
            title: "Long".into(),
            messages: (0..150)
                .map(|index| AiMessage::user(format!("message {index}")))
                .collect(),
        }];

        let (threads, _, _) = restore_ai_threads(&saved);
        assert_eq!(threads[0].messages.len(), 150);
    }

    #[test]
    fn restore_prefers_threads_and_repairs_active_and_next_ids() {
        let mut saved = PersistedState::default();
        saved.ai_messages = vec![AiMessage::user("ignored legacy")];
        saved.ai_threads = vec![
            AiThread {
                id: 4,
                title: "First".into(),
                messages: vec![AiMessage::user("a")],
            },
            AiThread {
                id: 8,
                title: String::new(),
                messages: vec![AiMessage::user("b")],
            },
        ];
        saved.ai_active_thread = 99;
        saved.next_ai_thread_id = 2;

        let (threads, active, next) = restore_ai_threads(&saved);

        assert_eq!(threads.len(), 2);
        assert_eq!(threads[1].title, "b");
        assert_eq!(active, 4);
        assert_eq!(next, 9);
    }
}
