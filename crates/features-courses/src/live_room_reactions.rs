// crates/features-courses/src/live_room_reactions.rs
//! Emoji reactions for the live room — a sender bar plus a floating-burst
//! overlay. Shared by the student (`live_room_view`) and teacher
//! (`live_room_broadcast`) screens so both look and behave the same.
//!
//! Reactions are ephemeral: the server validates against a fixed whitelist,
//! rate-limits, and broadcasts; clients render a brief rise-and-fade animation.
//! We avoid timers entirely — the active list is capped and the CSS animation
//! fades each emoji out, so old reactions simply scroll off as new ones arrive.

use dioxus::prelude::*;

/// The whitelist of reaction emoji. MUST match the server's `ALLOWED` set in
/// `handlers/live_sessions.rs` (same code points) or the server drops them.
pub const REACTION_EMOJI: [&str; 6] = [
    "\u{1f44d}",        // 👍
    "\u{2764}\u{fe0f}", // ❤️
    "\u{1f389}",        // 🎉
    "\u{1f44f}",        // 👏
    "\u{1f602}",        // 😂
    "\u{1f64c}",        // 🙌
];

/// Max simultaneously-rendered floating reactions (oldest drained first).
pub const MAX_ACTIVE_REACTIONS: usize = 12;

/// A single in-flight floating reaction. `id` is a per-room monotonic sequence
/// (stable key so the CSS animation isn't restarted on re-render); `lane`
/// spreads bursts horizontally so they don't all overlap.
#[derive(Clone, PartialEq, Debug)]
pub struct FloatingReaction {
    pub id: u64,
    pub emoji: String,
    pub lane: u8,
}

/// Append a reaction to the active list, assigning it the next sequence id and
/// a spread lane, and capping the list to `MAX_ACTIVE_REACTIONS`.
pub fn push_reaction(list: &mut Vec<FloatingReaction>, seq: &mut u64, emoji: String) {
    let id = *seq;
    *seq = seq.wrapping_add(1);
    let lane = (id % 5) as u8;
    list.push(FloatingReaction { id, emoji, lane });
    let len = list.len();
    if len > MAX_ACTIVE_REACTIONS {
        list.drain(..len - MAX_ACTIVE_REACTIONS);
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ReactionBarProps {
    pub on_react: EventHandler<String>,
}

/// The row of emoji buttons. Tapping one fires `on_react(emoji)`.
#[allow(non_snake_case)]
pub fn ReactionBar(props: ReactionBarProps) -> Element {
    rsx! {
        div { class: "live-reaction-bar", role: "group", "aria-label": "Send a reaction",
            for emoji in REACTION_EMOJI {
                button {
                    r#type: "button",
                    class: "live-reaction-btn",
                    title: "React {emoji}",
                    onclick: move |_| props.on_react.call(emoji.to_string()),
                    "{emoji}"
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct ReactionFloatsProps {
    pub reactions: Vec<FloatingReaction>,
}

/// The overlay that renders the floating-burst emojis. Each animates once
/// (rise + fade) via CSS; the stable `id` key keeps it from restarting.
#[allow(non_snake_case)]
pub fn ReactionFloats(props: ReactionFloatsProps) -> Element {
    rsx! {
        div { class: "live-reaction-floats", "aria-hidden": "true",
            for r in props.reactions.iter() {
                span {
                    key: "{r.id}",
                    class: "live-reaction-float live-reaction-float--lane{r.lane}",
                    "{r.emoji}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_assigns_sequential_ids_and_lanes() {
        let mut list = Vec::new();
        let mut seq = 0u64;
        push_reaction(&mut list, &mut seq, "👍".into());
        push_reaction(&mut list, &mut seq, "🎉".into());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, 0);
        assert_eq!(list[1].id, 1);
        assert_eq!(seq, 2);
        assert!(list[0].lane < 5 && list[1].lane < 5);
    }

    #[test]
    fn push_caps_active_list_oldest_first() {
        let mut list = Vec::new();
        let mut seq = 0u64;
        for _ in 0..(MAX_ACTIVE_REACTIONS + 3) {
            push_reaction(&mut list, &mut seq, "👏".into());
        }
        assert_eq!(list.len(), MAX_ACTIVE_REACTIONS);
        // Oldest (ids 0,1,2) dropped; the window starts at id 3.
        assert_eq!(list.first().unwrap().id, 3);
    }

    #[test]
    fn bar_renders_all_emoji_buttons() {
        fn app() -> Element {
            rsx! { ReactionBar { on_react: move |_| {} } }
        }
        let mut vdom = VirtualDom::new(app);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("live-reaction-bar"));
        for emoji in REACTION_EMOJI {
            assert!(html.contains(emoji), "missing {emoji}: {html}");
        }
    }
}
