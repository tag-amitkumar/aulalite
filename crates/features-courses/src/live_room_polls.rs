// crates/features-courses/src/live_room_polls.rs
//! In-class live polls for the live room — a teacher composer (question + 2..6
//! options + Start/End) and a voter card with a live bar-chart of results,
//! shared by the student (`live_room_view`) and teacher (`live_room_broadcast`)
//! screens so both render the same poll.
//!
//! Polls are ephemeral, driven entirely over the live-room socket (mirrors
//! reactions/cursors): the teacher sends `start_poll` / `end_poll`, anyone
//! sends `poll_vote`, and the server fans `poll_started` / `poll_results` /
//! `poll_ended` back to the room. This module owns the client-side poll state
//! machine (`PollClientState`) plus the two components; the views translate the
//! socket events into `apply_*` calls and forward the button callbacks onto the
//! socket.

use dioxus::prelude::*;

/// Option-count bounds, mirroring `core_types::live_room` /
/// `backend::services::live_room`. The composer enforces the same range so the
/// client never sends a shape the server will reject.
pub const POLL_MIN_OPTIONS: usize = 2;
pub const POLL_MAX_OPTIONS: usize = 6;

/// The active poll as the client knows it. `None` between polls.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PollClientState {
    pub active: Option<ActivePoll>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ActivePoll {
    pub poll_id: String,
    pub question: String,
    pub options: Vec<String>,
    /// `counts[i]` is the live tally for `options[i]`. Starts all-zero and is
    /// replaced wholesale by each `poll_results` / `poll_ended` event.
    pub counts: Vec<u32>,
    /// The option index this client voted for, if any. Drives the disabled
    /// state of the vote buttons and the "your vote" highlight.
    pub my_vote: Option<usize>,
    /// `true` once the poll has ended — the card freezes on final results.
    pub ended: bool,
}

impl PollClientState {
    /// Handle a `poll_started` event: replace any prior poll with a fresh,
    /// not-yet-voted, not-yet-ended poll.
    pub fn on_started(&mut self, poll_id: String, question: String, options: Vec<String>) {
        let n = options.len();
        self.active = Some(ActivePoll {
            poll_id,
            question,
            options,
            counts: vec![0; n],
            my_vote: None,
            ended: false,
        });
    }

    /// Handle a `poll_results` event: update the live tally if it targets the
    /// active poll. Ignores stale events for a different / ended poll.
    pub fn on_results(&mut self, poll_id: &str, counts: Vec<u32>) {
        if let Some(p) = self.active.as_mut() {
            if p.poll_id == poll_id && !p.ended {
                p.counts = counts;
            }
        }
    }

    /// Handle a `poll_ended` event: freeze the active poll on its final counts.
    pub fn on_ended(&mut self, poll_id: &str, counts: Vec<u32>) {
        if let Some(p) = self.active.as_mut() {
            if p.poll_id == poll_id {
                p.counts = counts;
                p.ended = true;
            }
        }
    }

    /// Record that this client voted for `option_index` (optimistic local
    /// state; the authoritative tally still arrives via `poll_results`). No-op
    /// if there is no active poll, it has ended, the user already voted, or the
    /// index is out of range.
    pub fn record_local_vote(&mut self, option_index: usize) -> bool {
        if let Some(p) = self.active.as_mut() {
            if !p.ended && p.my_vote.is_none() && option_index < p.options.len() {
                p.my_vote = Some(option_index);
                return true;
            }
        }
        false
    }
}

/// Total votes across all options, used to compute bar percentages.
pub fn total_votes(counts: &[u32]) -> u32 {
    counts.iter().copied().sum()
}

/// Integer percentage (0..=100) of `count` against `total`, guarding /0.
pub fn percent(count: u32, total: u32) -> u32 {
    if total == 0 {
        0
    } else {
        ((count as u64 * 100) / total as u64) as u32
    }
}

// ===========================================================================
// Voter card + live results — rendered in BOTH the student and teacher views.
// ===========================================================================

#[derive(Props, Clone, PartialEq)]
pub struct PollVoterCardProps {
    pub poll: ActivePoll,
    /// Fires `on_vote(option_index)` when the user taps an option. The parent
    /// forwards it onto the socket as `poll_vote` and records the local vote.
    pub on_vote: EventHandler<usize>,
}

/// The participant-facing poll card: the question, a vote button per option
/// (or a non-interactive bar once voted / ended), and a live bar chart.
#[allow(non_snake_case)]
pub fn PollVoterCard(props: PollVoterCardProps) -> Element {
    let poll = &props.poll;
    let total = total_votes(&poll.counts);
    // Show results (bars) once the user has voted or the poll has ended;
    // otherwise show clickable options.
    let show_results = poll.ended || poll.my_vote.is_some();

    rsx! {
        section { class: "live-poll-card", "aria-label": "Class poll",
            div { class: "live-poll-header",
                span { class: "live-poll-eyebrow",
                    if poll.ended { "Poll · final results" } else { "Live poll" }
                }
                if !poll.ended {
                    span { class: "live-poll-count", "{total} votes" }
                }
            }
            h4 { class: "live-poll-question", "{poll.question}" }
            ul { class: "live-poll-options", role: "list",
                for (i, option) in poll.options.iter().enumerate() {
                    {
                        let count = poll.counts.get(i).copied().unwrap_or(0);
                        let pct = percent(count, total);
                        let is_mine = poll.my_vote == Some(i);
                        let mut li_class = String::from("live-poll-option");
                        if is_mine { li_class.push_str(" live-poll-option--mine"); }
                        let option_label = option.clone();
                        rsx! {
                            li { key: "{i}", class: "{li_class}",
                                if show_results {
                                    div { class: "live-poll-bar-row",
                                        div { class: "live-poll-bar-track",
                                            div {
                                                class: "live-poll-bar-fill",
                                                style: "width: {pct}%;",
                                                "aria-hidden": "true",
                                            }
                                            span { class: "live-poll-bar-label", "{option_label}" }
                                        }
                                        span { class: "live-poll-bar-pct", "{pct}% ({count})" }
                                        if is_mine {
                                            span { class: "live-poll-your-vote", "aria-label": "Your vote", "\u{2713}" }
                                        }
                                    }
                                } else {
                                    button {
                                        r#type: "button",
                                        class: "live-poll-vote-btn",
                                        onclick: move |_| props.on_vote.call(i),
                                        "{option_label}"
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if poll.ended {
                p { class: "live-poll-foot", "This poll has ended." }
            } else if poll.my_vote.is_some() {
                p { class: "live-poll-foot", "Thanks for voting — results update live." }
            }
        }
    }
}

// ===========================================================================
// Teacher composer — rendered only in the broadcast view.
// ===========================================================================

#[derive(Props, Clone, PartialEq)]
pub struct PollComposerProps {
    /// The currently-active poll, if any. When `Some`, the composer collapses
    /// to a live summary + "End poll" control instead of the editor.
    pub active: Option<ActivePoll>,
    /// Fires with `(question, options)` when the teacher taps Start. Options
    /// are already trimmed and validated to 2..=6 non-empty entries.
    pub on_start: EventHandler<(String, Vec<String>)>,
    /// Fires with the poll id when the teacher taps End.
    pub on_end: EventHandler<String>,
}

/// Teacher-only poll composer. When no poll is active it renders the editor
/// (question + dynamic option rows + Start); while a poll runs it shows the
/// live tally and an End button.
#[allow(non_snake_case)]
pub fn PollComposer(props: PollComposerProps) -> Element {
    // Local draft state for the editor. Two empty options to start.
    let mut question = use_signal(String::new);
    let mut options = use_signal(|| vec![String::new(), String::new()]);

    // When a poll is live, show the running summary + End control.
    if let Some(poll) = &props.active {
        let poll_id = poll.poll_id.clone();
        let total = total_votes(&poll.counts);
        return rsx! {
            section { class: "live-poll-composer live-poll-composer--running", "aria-label": "Active poll",
                div { class: "live-poll-header",
                    span { class: "live-poll-eyebrow", "Poll running" }
                    span { class: "live-poll-count", "{total} votes" }
                }
                h4 { class: "live-poll-question", "{poll.question}" }
                ul { class: "live-poll-options", role: "list",
                    for (i, option) in poll.options.iter().enumerate() {
                        {
                            let count = poll.counts.get(i).copied().unwrap_or(0);
                            let pct = percent(count, total);
                            let option_label = option.clone();
                            rsx! {
                                li { key: "{i}", class: "live-poll-option",
                                    div { class: "live-poll-bar-row",
                                        div { class: "live-poll-bar-track",
                                            div {
                                                class: "live-poll-bar-fill",
                                                style: "width: {pct}%;",
                                                "aria-hidden": "true",
                                            }
                                            span { class: "live-poll-bar-label", "{option_label}" }
                                        }
                                        span { class: "live-poll-bar-pct", "{pct}% ({count})" }
                                    }
                                }
                            }
                        }
                    }
                }
                design_system::Button {
                    label: "End poll".to_string(),
                    variant: design_system::ButtonVariant::Secondary,
                    size: design_system::ButtonSize::Sm,
                    on_click: move |_| props.on_end.call(poll_id.clone()),
                }
            }
        };
    }

    // Editor mode. Start is enabled only when the draft is valid.
    let q_trimmed = question.read().trim().to_string();
    let trimmed_opts: Vec<String> = options
        .read()
        .iter()
        .map(|o| o.trim().to_string())
        .filter(|o| !o.is_empty())
        .collect();
    let can_start = !q_trimmed.is_empty()
        && (POLL_MIN_OPTIONS..=POLL_MAX_OPTIONS).contains(&trimmed_opts.len());
    let can_add = options.read().len() < POLL_MAX_OPTIONS;
    let can_remove = options.read().len() > POLL_MIN_OPTIONS;
    let opt_count = options.read().len();

    let on_start_click = move |_| {
        let q = question.read().trim().to_string();
        let opts: Vec<String> = options
            .read()
            .iter()
            .map(|o| o.trim().to_string())
            .filter(|o| !o.is_empty())
            .collect();
        if q.is_empty() || !(POLL_MIN_OPTIONS..=POLL_MAX_OPTIONS).contains(&opts.len()) {
            return;
        }
        props.on_start.call((q, opts));
        // Reset the draft for the next poll.
        question.set(String::new());
        options.set(vec![String::new(), String::new()]);
    };

    rsx! {
        section { class: "live-poll-composer", "aria-label": "Create a poll",
            div { class: "live-poll-header",
                span { class: "live-poll-eyebrow", "New poll" }
            }
            label { class: "live-poll-field-label", "Question" }
            design_system::Input {
                value: question.read().clone(),
                placeholder: "Ask the class a question…".to_string(),
                on_input: move |v: String| question.set(v),
            }
            div { class: "live-poll-options-editor",
                for i in 0..opt_count {
                    div { key: "{i}", class: "live-poll-option-edit",
                        design_system::Input {
                            value: options.read().get(i).cloned().unwrap_or_default(),
                            placeholder: format!("Option {}", i + 1),
                            on_input: move |v: String| {
                                let mut o = options.write();
                                if let Some(slot) = o.get_mut(i) {
                                    *slot = v;
                                }
                            },
                        }
                        if can_remove {
                            button {
                                r#type: "button",
                                class: "live-poll-option-remove",
                                title: "Remove option",
                                "aria-label": "Remove option",
                                onclick: move |_| {
                                    let mut o = options.write();
                                    if o.len() > POLL_MIN_OPTIONS {
                                        o.remove(i);
                                    }
                                },
                                "\u{00d7}"
                            }
                        }
                    }
                }
            }
            div { class: "live-poll-composer-actions",
                if can_add {
                    design_system::Button {
                        label: "Add option".to_string(),
                        variant: design_system::ButtonVariant::Ghost,
                        size: design_system::ButtonSize::Sm,
                        on_click: move |_| options.write().push(String::new()),
                    }
                }
                design_system::Button {
                    label: "Start poll".to_string(),
                    variant: design_system::ButtonVariant::Primary,
                    size: design_system::ButtonSize::Sm,
                    disabled: !can_start,
                    on_click: on_start_click,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started_state() -> PollClientState {
        let mut s = PollClientState::default();
        s.on_started(
            "p1".into(),
            "Q?".into(),
            vec!["A".into(), "B".into(), "C".into()],
        );
        s
    }

    #[test]
    fn on_started_initializes_zeroed_counts() {
        let s = started_state();
        let p = s.active.unwrap();
        assert_eq!(p.counts, vec![0, 0, 0]);
        assert_eq!(p.my_vote, None);
        assert!(!p.ended);
    }

    #[test]
    fn on_results_updates_only_matching_active_poll() {
        let mut s = started_state();
        s.on_results("p1", vec![2, 1, 0]);
        assert_eq!(s.active.as_ref().unwrap().counts, vec![2, 1, 0]);
        // A stale event for a different poll id is ignored.
        s.on_results("other", vec![9, 9, 9]);
        assert_eq!(s.active.as_ref().unwrap().counts, vec![2, 1, 0]);
    }

    #[test]
    fn on_ended_freezes_final_counts_and_ignores_later_results() {
        let mut s = started_state();
        s.on_ended("p1", vec![3, 1, 0]);
        let p = s.active.as_ref().unwrap();
        assert!(p.ended);
        assert_eq!(p.counts, vec![3, 1, 0]);
        // Results after end must not move a frozen poll.
        s.on_results("p1", vec![5, 5, 5]);
        assert_eq!(s.active.as_ref().unwrap().counts, vec![3, 1, 0]);
    }

    #[test]
    fn record_local_vote_is_once_only_and_bounds_checked() {
        let mut s = started_state();
        assert!(s.record_local_vote(1));
        assert_eq!(s.active.as_ref().unwrap().my_vote, Some(1));
        // Second vote rejected.
        assert!(!s.record_local_vote(0));
        assert_eq!(s.active.as_ref().unwrap().my_vote, Some(1));

        // Out-of-range on a fresh poll is rejected.
        let mut s2 = started_state();
        assert!(!s2.record_local_vote(9));
        assert_eq!(s2.active.as_ref().unwrap().my_vote, None);

        // Voting on an ended poll is rejected.
        let mut s3 = started_state();
        s3.on_ended("p1", vec![0, 0, 0]);
        assert!(!s3.record_local_vote(0));
    }

    #[test]
    fn percent_guards_zero_total_and_rounds_down() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(1, 0), 0);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(2, 3), 66);
        assert_eq!(percent(3, 3), 100);
        assert_eq!(total_votes(&[1, 2, 3]), 6);
    }

    #[test]
    fn voter_card_renders_options_before_vote() {
        fn app() -> Element {
            rsx! {
                PollVoterCard {
                    poll: ActivePoll {
                        poll_id: "p1".into(),
                        question: "Favorite?".into(),
                        options: vec!["Rust".into(), "Go".into()],
                        counts: vec![0, 0],
                        my_vote: None,
                        ended: false,
                    },
                    on_vote: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Favorite?"));
        assert!(
            html.contains("live-poll-vote-btn"),
            "vote buttons missing: {html}"
        );
        assert!(html.contains("Rust") && html.contains("Go"));
    }

    #[test]
    fn voter_card_renders_bars_after_vote() {
        fn app() -> Element {
            rsx! {
                PollVoterCard {
                    poll: ActivePoll {
                        poll_id: "p1".into(),
                        question: "Favorite?".into(),
                        options: vec!["Rust".into(), "Go".into()],
                        counts: vec![3, 1],
                        my_vote: Some(0),
                        ended: false,
                    },
                    on_vote: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        // No vote buttons once voted; bars instead.
        assert!(
            !html.contains("live-poll-vote-btn"),
            "should hide vote buttons: {html}"
        );
        assert!(html.contains("live-poll-bar-fill"), "bars missing: {html}");
        assert!(
            html.contains("width: 75%"),
            "expected 75% bar for 3/4: {html}"
        );
    }

    #[test]
    fn composer_editor_renders_when_no_active_poll() {
        fn app() -> Element {
            rsx! {
                PollComposer {
                    active: None,
                    on_start: move |_| {},
                    on_end: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("Start poll"), "start button missing: {html}");
        assert!(html.contains("Add option"), "add-option missing: {html}");
    }

    #[test]
    fn composer_shows_running_summary_with_end_when_active() {
        fn app() -> Element {
            rsx! {
                PollComposer {
                    active: Some(ActivePoll {
                        poll_id: "p1".into(),
                        question: "Live?".into(),
                        options: vec!["Yes".into(), "No".into()],
                        counts: vec![2, 0],
                        my_vote: None,
                        ended: false,
                    }),
                    on_start: move |_| {},
                    on_end: move |_| {},
                }
            }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("End poll"), "end button missing: {html}");
        assert!(html.contains("Live?"));
        assert!(
            !html.contains("Start poll"),
            "should not show editor while running: {html}"
        );
    }
}
