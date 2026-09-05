//! Flashcard surfaces (learning-suite Cycle 6).
//!
//! `CourseFlashcardsTab` owns the tab with three internal views:
//! deck list (student: published decks + due counts; staff: all decks +
//! authoring) → deck editor (staff: add/edit/delete cards, publish) →
//! review session (kinetics `FlashcardDeck`: flip + Again/Hard/Good/Easy;
//! ratings persist server-side via SM-2, "again" cards loop back into the
//! session queue).

use crate::api::{self, FlashcardCardDto, FlashcardDeckDto, ReviewCardDto};
use design_system::kinetics_ui::{Flashcard, FlashcardDeck, ReviewRating};
use design_system::{Button, ButtonVariant, Card, EmptyState, Field, Input, SkeletonCard};
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
enum View {
    List,
    Edit(String),
    Review(String),
}

fn rating_str(r: ReviewRating) -> &'static str {
    match r {
        ReviewRating::Again => "again",
        ReviewRating::Hard => "hard",
        ReviewRating::Good => "good",
        ReviewRating::Easy => "easy",
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CourseFlashcardsTabProps {
    pub course_id: String,
    /// Whether the current user can author decks (course staff).
    pub can_edit: bool,
}

#[component]
pub fn CourseFlashcardsTab(props: CourseFlashcardsTabProps) -> Element {
    let view = use_signal(|| View::List);
    let course_id = props.course_id.clone();
    let current = view.read().clone();
    match current {
        View::List => rsx! {
            DeckList { course_id, can_edit: props.can_edit, view }
        },
        View::Edit(deck_id) => rsx! {
            DeckEditor { course_id, deck_id, view }
        },
        View::Review(deck_id) => rsx! {
            ReviewSession { course_id, deck_id, view }
        },
    }
}

#[derive(Props, Clone, PartialEq)]
struct DeckListProps {
    course_id: String,
    can_edit: bool,
    view: Signal<View>,
}

#[component]
fn DeckList(props: DeckListProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let view = props.view;
    let decks = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::list_flashcard_decks(&cx, &course_id).await }
        }
    });

    let new_deck = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        move |_| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let mut view = view;
            spawn(async move {
                if let Ok(deck) =
                    api::create_flashcard_deck(&cx, &course_id, "Untitled deck", None).await
                {
                    view.set(View::Edit(deck.id));
                }
            });
        }
    };

    let snap = decks.read_unchecked();
    let body: Element = match snap.as_ref() {
        Some(Ok(rows)) if rows.is_empty() => rsx! {
            EmptyState {
                title: "No flashcard decks yet.".to_string(),
                description: if props.can_edit {
                    "Create a deck and add cards — students review them on an SM-2 spaced schedule.".to_string()
                } else {
                    "Your teacher hasn't published any flashcard decks for this course yet.".to_string()
                },
            }
        },
        Some(Ok(rows)) => rsx! {
            div { class: "flashcard-deck-list",
                for deck in rows.clone() {
                    DeckListRow {
                        key: "{deck.id}",
                        deck: deck.clone(),
                        can_edit: props.can_edit,
                        course_id: props.course_id.clone(),
                        view,
                    }
                }
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load decks: {e}" } },
        None => rsx! { SkeletonCard { height: "160px".to_string() } },
    };
    drop(snap);

    rsx! {
        section { class: "course-flashcards",
            if props.can_edit {
                div { class: "flashcards-toolbar",
                    Button {
                        label: "+ New deck".to_string(),
                        variant: ButtonVariant::Primary,
                        on_click: new_deck,
                    }
                }
            }
            { body }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct DeckListRowProps {
    deck: FlashcardDeckDto,
    can_edit: bool,
    course_id: String,
    view: Signal<View>,
}

#[component]
fn DeckListRow(props: DeckListRowProps) -> Element {
    let cx = api::use_api();
    let deck = props.deck.clone();
    let mut view = props.view;
    // Due count is decorative; fetched lazily per deck for students.
    let due = use_resource({
        let cx = cx.clone();
        let course_id = props.course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::get_flashcards_due_count(&cx, &course_id).await }
        }
    });
    let due_label = {
        let snap = due.read_unchecked();
        match snap.as_ref() {
            Some(Ok(d)) if d.due > 0 => format!("Review ({} due)", d.due),
            _ => "Review".to_string(),
        }
    };

    let deck_id_review = deck.id.clone();
    let deck_id_edit = deck.id.clone();
    rsx! {
        Card {
            div { class: "flashcard-deck-row",
                div { class: "flashcard-deck-meta",
                    h3 { class: "flashcard-deck-title", "{deck.title}" }
                    p { class: "muted",
                        "{deck.card_count} card(s)"
                        if deck.status == "draft" { " · draft" }
                    }
                    if let Some(desc) = deck.description.clone().filter(|d| !d.is_empty()) {
                        p { class: "flashcard-deck-desc", "{desc}" }
                    }
                }
                div { class: "flashcard-deck-actions",
                    if deck.card_count > 0 {
                        Button {
                            label: due_label.clone(),
                            variant: ButtonVariant::Primary,
                            on_click: move |_| view.set(View::Review(deck_id_review.clone())),
                        }
                    }
                    if props.can_edit {
                        Button {
                            label: "Edit".to_string(),
                            variant: ButtonVariant::Secondary,
                            on_click: move |_| view.set(View::Edit(deck_id_edit.clone())),
                        }
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct DeckEditorProps {
    course_id: String,
    deck_id: String,
    view: Signal<View>,
}

#[component]
fn DeckEditor(props: DeckEditorProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let deck_id = props.deck_id.clone();
    let mut view = props.view;

    let cards = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        let deck_id = deck_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let deck_id = deck_id.clone();
            async move { api::list_flashcards(&cx, &course_id, &deck_id).await }
        }
    });
    let decks = use_resource({
        let cx = cx.clone();
        let course_id = course_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            async move { api::list_flashcard_decks(&cx, &course_id).await }
        }
    });

    let mut front = use_signal(String::new);
    let mut back = use_signal(String::new);
    let error = use_signal(|| None::<String>);

    let add_card = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        let deck_id = deck_id.clone();
        move |_| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let deck_id = deck_id.clone();
            let mut cards = cards;
            let mut front = front;
            let mut back = back;
            let mut error = error;
            let f = front.read().trim().to_string();
            let b = back.read().trim().to_string();
            if f.is_empty() || b.is_empty() {
                error.set(Some("Both the front and the back are required.".into()));
                return;
            }
            spawn(async move {
                match api::create_flashcard(&cx, &course_id, &deck_id, &f, &b).await {
                    Ok(_) => {
                        front.set(String::new());
                        back.set(String::new());
                        error.set(None);
                        cards.restart();
                    }
                    Err(e) => error.set(Some(format!("{e}"))),
                }
            });
        }
    };

    let this_deck: Option<FlashcardDeckDto> = {
        let snap = decks.read_unchecked();
        match snap.as_ref() {
            Some(Ok(rows)) => rows.iter().find(|d| d.id == deck_id).cloned(),
            _ => None,
        }
    };
    let publish_toggle: Element = match &this_deck {
        Some(deck) => {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let deck_id = deck_id.clone();
            let next_status = if deck.status == "published" {
                "draft"
            } else {
                "published"
            };
            let label = if deck.status == "published" {
                "Unpublish".to_string()
            } else {
                "Publish".to_string()
            };
            rsx! {
                Button {
                    label,
                    variant: ButtonVariant::Secondary,
                    on_click: move |_| {
                        let cx = cx.clone();
                        let course_id = course_id.clone();
                        let deck_id = deck_id.clone();
                        let mut decks = decks;
                        spawn(async move {
                            let _ = api::patch_flashcard_deck(
                                &cx, &course_id, &deck_id, None, None, Some(next_status),
                            )
                            .await;
                            decks.restart();
                        });
                    },
                }
            }
        }
        None => rsx! {},
    };

    let cards_snap = cards.read_unchecked();
    let card_list: Element = match cards_snap.as_ref() {
        Some(Ok(rows)) if rows.is_empty() => rsx! {
            p { class: "muted", "No cards yet — add the first one below." }
        },
        Some(Ok(rows)) => rsx! {
            ol { class: "flashcard-card-list",
                for card in rows.clone() {
                    EditorCardRow {
                        key: "{card.id}",
                        card: card.clone(),
                        course_id: course_id.clone(),
                        deck_id: deck_id.clone(),
                        on_changed: move |_| {
                            let mut cards = cards;
                            cards.restart();
                        },
                    }
                }
            }
        },
        Some(Err(e)) => rsx! { p { class: "error", "Could not load cards: {e}" } },
        None => rsx! { SkeletonCard { height: "120px".to_string() } },
    };
    drop(cards_snap);

    let title = this_deck
        .as_ref()
        .map(|d| d.title.clone())
        .unwrap_or_else(|| "Deck".to_string());

    rsx! {
        section { class: "flashcard-deck-editor",
            div { class: "flashcards-toolbar",
                Button {
                    label: "← Back to decks".to_string(),
                    variant: ButtonVariant::Ghost,
                    on_click: move |_| view.set(View::List),
                }
                { publish_toggle }
            }
            h3 { class: "flashcard-deck-title", "{title}" }
            { card_list }
            Card {
                div { class: "flashcard-add-form",
                    Field {
                        label: "Front".to_string(),
                        Input {
                            value: front.read().clone(),
                            placeholder: "Prompt — e.g. What is the derivative of x²?".to_string(),
                            on_input: move |v| front.set(v),
                        }
                    }
                    Field {
                        label: "Back".to_string(),
                        Input {
                            value: back.read().clone(),
                            placeholder: "Answer — e.g. 2x".to_string(),
                            on_input: move |v| back.set(v),
                        }
                    }
                    if let Some(e) = error.read().clone() {
                        p { class: "error", "{e}" }
                    }
                    Button {
                        label: "Add card".to_string(),
                        variant: ButtonVariant::Primary,
                        on_click: add_card,
                    }
                }
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct EditorCardRowProps {
    card: FlashcardCardDto,
    course_id: String,
    deck_id: String,
    on_changed: EventHandler<()>,
}

#[component]
fn EditorCardRow(props: EditorCardRowProps) -> Element {
    let cx = api::use_api();
    let card = props.card.clone();
    let on_changed = props.on_changed;
    let course_id = props.course_id.clone();
    let deck_id = props.deck_id.clone();
    let card_id = card.id.clone();
    rsx! {
        li { class: "flashcard-card-row",
            span { class: "flashcard-card-front", "{card.front}" }
            span { class: "flashcard-card-back muted", "{card.back}" }
            button {
                class: "lesson-delete",
                r#type: "button",
                "aria-label": "Delete card",
                title: "Delete card",
                onclick: move |_| {
                    let cx = cx.clone();
                    let course_id = course_id.clone();
                    let deck_id = deck_id.clone();
                    let card_id = card_id.clone();
                    spawn(async move {
                        if api::delete_flashcard(&cx, &course_id, &deck_id, &card_id).await.is_ok() {
                            on_changed.call(());
                        }
                    });
                },
                "\u{00d7}"
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
struct ReviewSessionProps {
    course_id: String,
    deck_id: String,
    view: Signal<View>,
}

#[component]
fn ReviewSession(props: ReviewSessionProps) -> Element {
    let cx = api::use_api();
    let course_id = props.course_id.clone();
    let deck_id = props.deck_id.clone();
    let mut view = props.view;

    // The session queue is loaded once; "again" cards re-enter the back of
    // the queue, every other rating advances. `reviewed` counts ratings.
    let mut queue = use_signal(Vec::<ReviewCardDto>::new);
    let mut loaded = use_signal(|| false);
    let mut load_error = use_signal(|| None::<String>);
    let mut flipped = use_signal(|| false);
    let reviewed = use_signal(|| 0usize);

    use_future({
        let cx = cx.clone();
        let course_id = course_id.clone();
        let deck_id = deck_id.clone();
        move || {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let deck_id = deck_id.clone();
            async move {
                match api::get_review_queue(&cx, &course_id, &deck_id).await {
                    Ok(cards) => {
                        queue.set(cards);
                        loaded.set(true);
                    }
                    Err(e) => load_error.set(Some(format!("{e}"))),
                }
            }
        }
    });

    let on_rate = {
        let cx = cx.clone();
        let course_id = course_id.clone();
        let deck_id = deck_id.clone();
        move |rating: ReviewRating| {
            let cx = cx.clone();
            let course_id = course_id.clone();
            let deck_id = deck_id.clone();
            let mut queue = queue;
            let mut flipped = flipped;
            let mut reviewed = reviewed;
            let Some(card) = queue.read().first().cloned() else {
                return;
            };
            flipped.set(false);
            let done = *reviewed.read();
            reviewed.set(done + 1);
            // Optimistic queue advance; persistence is fire-and-forget (a
            // failed write costs one repetition, never the session).
            {
                let mut q = queue.write();
                q.remove(0);
                if rating == ReviewRating::Again {
                    q.push(card.clone());
                }
            }
            spawn(async move {
                let _ = api::record_flashcard_review(
                    &cx,
                    &course_id,
                    &deck_id,
                    &card.id,
                    rating_str(rating),
                )
                .await;
            });
        }
    };

    let body: Element = if let Some(e) = load_error.read().clone() {
        rsx! { p { class: "error", "Could not load the review queue: {e}" } }
    } else if !*loaded.read() {
        rsx! { SkeletonCard { height: "260px".to_string() } }
    } else {
        let cards: Vec<Flashcard> = queue
            .read()
            .iter()
            .map(|c| Flashcard::new(c.id.clone(), c.front.clone(), c.back.clone()))
            .collect();
        let done = *reviewed.read();
        if cards.is_empty() {
            rsx! {
                EmptyState {
                    title: if done == 1 {
                        "Session complete — 1 review recorded.".to_string()
                    } else if done > 1 {
                        format!("Session complete — {done} reviews recorded.")
                    } else {
                        "Nothing due right now.".to_string()
                    },
                    description: "Cards come back on their SM-2 schedule: tomorrow for new material, later and later as you keep getting them right.".to_string(),
                }
            }
        } else {
            rsx! {
                FlashcardDeck {
                    cards,
                    index: 0,
                    flipped: *flipped.read(),
                    on_flip: move |f: bool| flipped.set(f),
                    on_rate,
                }
            }
        }
    };

    rsx! {
        section { class: "flashcard-review",
            div { class: "flashcards-toolbar",
                Button {
                    label: "← Back to decks".to_string(),
                    variant: ButtonVariant::Ghost,
                    on_click: move |_| view.set(View::List),
                }
            }
            { body }
        }
    }
}

#[cfg(test)]
mod ssr_tests {
    use super::*;

    #[test]
    fn rating_strings_match_backend_vocabulary() {
        assert_eq!(rating_str(ReviewRating::Again), "again");
        assert_eq!(rating_str(ReviewRating::Hard), "hard");
        assert_eq!(rating_str(ReviewRating::Good), "good");
        assert_eq!(rating_str(ReviewRating::Easy), "easy");
    }

    #[test]
    fn tab_defaults_to_deck_list_with_authoring_for_staff() {
        #[component]
        fn Harness() -> Element {
            use crate::api::ApiContext;
            let api_signal = use_signal(|| ApiContext {
                base_url: String::new(),
                id_token: String::new(),
            });
            use_context_provider::<Signal<ApiContext>>(|| api_signal);
            rsx! {
                CourseFlashcardsTab { course_id: "c1".to_string(), can_edit: true }
            }
        }
        let mut vdom = VirtualDom::new(Harness);
        vdom.rebuild_in_place();
        let html = dioxus_ssr::render(&vdom);
        assert!(html.contains("New deck"), "authoring CTA missing: {html}");
    }
}
