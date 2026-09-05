//! Gamification surfaces (learning-suite Cycle 4): the dashboard XP/streak
//! strip with achievement-unlock celebrations, and the per-course
//! leaderboard view with the self opt-out toggle.

use crate::api::{self, GamificationDto};
use design_system::kinetics_ui::{
    AchievementUnlock, Leaderboard, LeaderboardEntry, StreakBadge, XpBar,
};
use design_system::{SkeletonCard, Switch};
use dioxus::prelude::*;

/// Dashboard strip: XP bar + streak chip; pops an `AchievementUnlock`
/// celebration for the newest unseen unlock (dismiss marks all seen).
/// Renders nothing for users with no XP yet (teachers, fresh accounts).
#[component]
pub fn GamifyStrip() -> Element {
    let cx = api::use_api();
    let mut gamification = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::get_my_gamification(&cx).await }
        }
    });

    let snap = gamification.read_unchecked();
    let data: GamificationDto = match snap.as_ref() {
        Some(Ok(d)) => d.clone(),
        // Errors (or loading) just hide the strip — it's decorative.
        _ => return rsx! {},
    };
    drop(snap);

    if data.total_xp == 0 {
        return rsx! {};
    }

    let unseen = data.unlocks.iter().find(|u| !u.seen).cloned();
    let dismiss = {
        let cx = cx.clone();
        move |_| {
            let cx = cx.clone();
            spawn(async move {
                let _ = api::mark_gamification_seen(&cx).await;
                gamification.restart();
            });
        }
    };

    rsx! {
        div { class: "gamify-strip",
            XpBar {
                level: data.level,
                current_xp: data.xp_into_level.max(0) as u32,
                next_level_xp: data.level_span.max(1) as u32,
            }
            StreakBadge {
                days: data.current_streak_days.max(0) as u32,
                active: data.streak_active_today,
            }
            if let Some(unlock) = unseen {
                AchievementUnlock {
                    title: unlock.title,
                    description: unlock.description,
                    on_dismiss: Some(EventHandler::new(dismiss)),
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseLeaderboardProps {
    pub course_id: String,
}

/// Per-course standings (top 50, opt-outs excluded) with a personal
/// "hide me from leaderboards" toggle (tenant-wide setting).
#[component]
pub fn CourseLeaderboard(props: CourseLeaderboardProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();

    let mut board = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::get_course_leaderboard(&cx, &course_id).await }
        }
    });
    let opt_out = use_resource({
        let cx = cx.clone();
        move || {
            let cx = cx.clone();
            async move { api::get_leaderboard_opt_out(&cx).await }
        }
    });

    let opted_out = {
        let snap = opt_out.read_unchecked();
        match snap.as_ref() {
            Some(Ok(o)) => o.opted_out,
            _ => false,
        }
    };
    let toggle_opt_out = {
        let cx = cx.clone();
        move |checked: bool| {
            let cx = cx.clone();
            spawn(async move {
                // Switch reports the NEW visibility; opted_out is its inverse.
                let _ = api::put_leaderboard_opt_out(&cx, !checked).await;
                board.restart();
            });
        }
    };

    let snap = board.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) if rows.is_empty() => rsx! {
            p { class: "muted", "No standings yet — XP appears as students complete lessons and quizzes." }
        },
        Some(Ok(rows)) => {
            let entries: Vec<LeaderboardEntry> = rows
                .iter()
                .map(|r| LeaderboardEntry {
                    name: if r.is_me {
                        format!("{} (you)", r.display_name)
                    } else {
                        r.display_name.clone()
                    },
                    score: format!("{} XP", r.course_xp),
                    highlight: r.is_me,
                })
                .collect();
            rsx! {
                Leaderboard { label: "Course leaderboard".to_string(), entries }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load the leaderboard: {e}" } },
        None => rsx! { SkeletonCard {} },
    };
    drop(snap);

    rsx! {
        section { class: "course-leaderboard",
            div { class: "course-leaderboard-controls",
                Switch {
                    checked: !opted_out,
                    on_change: toggle_opt_out,
                    label: "Show me on leaderboards".to_string(),
                }
            }
            {body}
        }
    }
}
