//! Phase 1a feature crate: courses, modules, lessons, enrollment, schedule.
//! Real screens added in subsequent tasks.
#![allow(
    dead_code,
    non_snake_case,
    clippy::doc_lazy_continuation,
    clippy::redundant_locals,
    clippy::type_complexity
)]

pub mod a11y;
pub use a11y::{use_focus_trap, FocusTrap, SkipToContent, MAIN_CONTENT_ID};
pub mod accept_invite;
pub mod active_session;
pub mod announcements;
pub mod api;
pub mod app_shell;
pub mod code_modal;
pub mod command_palette;
pub mod course_builder;
pub mod course_create;
pub mod course_detail;
pub mod course_list;
pub mod course_people;
pub mod course_progress;
pub mod dashboard;
pub mod error_messages;
pub mod file_asset_image;
pub mod invite_modal;
pub mod lesson_editor;
pub mod onboarding_wizard;
pub mod quiz_editor;
pub mod quiz_list;
pub mod quiz_take;
pub mod redeem_code;
pub mod schedule_view;
pub mod series_scheduler;
pub use file_asset_image::FileAssetImage;
pub mod analytics_panel;
pub mod browser_runtime;
pub mod bulk_import;
pub mod calendar_view;
pub use calendar_view::{CalendarEventDto, CalendarView};
pub mod catalog_view;
pub mod certificates_panel;
pub use catalog_view::CatalogView;
pub mod course_checklist;
pub mod course_syllabus;
pub use course_syllabus::{CourseSyllabusSettings, CourseSyllabusView};
pub mod discussions;
pub mod lesson_notes;
pub use lesson_notes::LessonNotes;
pub mod locale_switcher;
pub use locale_switcher::LocaleSwitcher;
pub mod peer_review;
pub mod scorm_player;
pub use scorm_player::{ScormPlayer, ScormTab};
pub mod security_settings;
pub use security_settings::SecuritySettings;
pub mod privacy_settings;
pub use privacy_settings::PrivacySettings;
pub mod transcript_view;
pub use transcript_view::TranscriptView;
pub mod file_picker;
pub mod flashcards_panel;
pub mod gamify_panel;
pub mod gradebook_panel;
pub mod lesson_outline_view;
pub mod rubric_editor;
pub use lesson_outline_view::{LessonOutlineView, LessonView};
pub mod course_cover_editor;
pub use course_cover_editor::CourseCoverEditor;
pub mod lesson_video_editor;
pub use lesson_video_editor::LessonVideoEditor;
pub mod lesson_files_editor;
pub use lesson_files_editor::LessonFilesEditor;
pub mod assignment_detail;
pub mod assignment_editor;
pub mod breakout_rooms;
pub mod live_now_banner;
pub mod live_room_broadcast;
pub mod live_room_health;
pub mod live_room_lobby;
#[cfg(not(target_arch = "wasm32"))]
pub mod live_room_native;
pub mod live_room_session;
pub mod live_room_shell;
pub mod live_room_socket;
pub mod live_room_view;
pub mod live_room_whiteboard;
pub mod markdown;
pub mod notification_bell;
pub use live_room_shell::{should_poll_for_start, CallerRole, LiveRoomShell, SessionStatus};
pub mod assignment_list;
pub mod attendance_panel;
pub mod live_room_audio_publisher;
pub mod live_room_chat;
pub mod live_room_capture;
pub mod live_room_devices;
pub mod live_room_hand_raise;
pub mod live_room_ice;
pub mod live_room_polls;
pub mod live_room_prejoin;
pub mod live_room_presence;
pub mod live_room_reactions;
pub mod live_room_replay;
pub mod live_room_stats;
pub mod live_room_video_fx;
pub mod live_room_whep;
pub mod live_room_whip;
pub mod recording_chapters;
pub mod session_feedback;
pub mod start_now_button;
pub mod start_now_modal;
pub mod submission_form;
pub mod submission_grade_modal;
pub mod submission_view;
pub mod submissions_grading_table;
