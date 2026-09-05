//! Curated re-exports of `dioxus-kinetics` components for AulaLite.
//!
//! The `kinetics::prelude` and AulaLite's own `design_system` top-level exports
//! share many names (`Button`, `Toast`, `Switch`, `Select`, `Sheet`, `Avatar`,
//! `Badge`, `EmptyState`, `Tabs`, `Tooltip`, `DropdownMenu`, `Checkbox`,
//! `Skeleton`, `Spinner`, …). To let a feature module freely write both
//! `use design_system::*;` and `use design_system::kinetics_ui::*;` without
//! ambiguous-import errors, this module re-exports **only** the kinetics
//! components AulaLite does NOT already provide — the genuine value-adds:
//! glass surfaces, metric cards, command palette, data tables, the motion /
//! Scene system, and the AI surfaces.
//!
//! For the colliding primitives, keep using AulaLite's `design_system`
//! versions; for everything below, prefer the kinetics version.

// ── Surfaces, layout & navigation value-adds ───────────────────────────────
pub use kinetics::prelude::{
    Accordion, AccordionSection, Alert, AlertTone, Breadcrumb, BreadcrumbItem, ContentPlane,
    GlassLayer, GlassSurface, Heading, IconButton, IconButtonSize, IconButtonTone, Pagination,
    Popover, PopoverSide, Progress, Sidebar, SidebarItem, SidebarSection, Slider, Stack, Stepper,
    StepperStep, Surface, Text, TextVariant, Toolbar,
};

// ── Selection / data controls value-adds ───────────────────────────────────
pub use kinetics::prelude::{
    Combobox, ComboboxOption, DataTable, DataTableColumn, DataTableRow, DatePicker, Dialog,
    DialogAction, DialogActionTone, MetricCard, MetricReadout, MetricTone, RadioGroup, RadioOption,
    SegmentItem, SegmentedControl, SortDirection,
};

// NOTE: kinetics' AI surfaces (AssistantPanel, StreamingText, CitationChip,
// AgentTimeline, PromptInput, SourceCard, …) are deliberately NOT re-exported.
// AulaLite stays AI-free; AI-assisted study is handled by the separate
// `elementors` system (https://github.com/ChiranjibChaudhuri/elementors), not
// embedded here. The components still compile inside the kinetics dependency;
// they're just not part of AulaLite's component surface.

// ── Command palette (Cmd-K) ────────────────────────────────────────────────
pub use crate::{CommandGroup, CommandItem, CommandMenu};
pub use kinetics::prelude::CommandFinder;

// ── Motion / cinematic system ──────────────────────────────────────────────
pub use kinetics::prelude::{
    Clip, Cue, KineticBox, KineticText, MotionPath, Presence, PresenceCue, PresenceGate, Scene,
    SceneContext, Sequence, SequenceContext, SharedElement, SharedLayout, SplitMode, SplitText,
    TimelineScope,
};

// ── Cinematic blocks (ui-blocks) ───────────────────────────────────────────
pub use kinetics::prelude::{
    Caption, LowerThird, LowerThirdAccent, MetricCounter, SocialOverlay, SocialPlatform,
    WipeTransition,
};

// ── Glass material model + layout/token helpers ────────────────────────────
pub use kinetics::prelude::{
    resolve_glass, Density, GlassDensity, GlassLevel, GlassPolicy, GlassRecipe, GlassRequest,
    GlassTone, Theme, ThemeMode,
};

// ── Motion runtime hooks (Scene driving, reduced-motion) ───────────────────
pub use kinetics::prelude::{
    use_animation_value, use_reduced_motion, SceneClock, SceneDriver, SceneState,
};

// ── Reactive theming (ui-runtime) ──────────────────────────────────────────
// ThemeProvider resolves `prefers-color-scheme` + `data-ui-theme`/`data-ui-density`
// attributes reactively; the hooks read the resolved mode/density from context.
pub use kinetics::prelude::{use_density, use_theme_mode, ThemeProvider};

// ── Charts ─────────────────────────────────────────────────────────────────
pub use kinetics::prelude::{BarChart, ChartSeries, ChartTone, DonutGauge, LineChart, Sparkline};

// ── Sortable (drag-to-reorder, keyboard accessible) ────────────────────────
pub use kinetics::prelude::{
    apply_kanban_move, KanbanBoard, KanbanColumn, KanbanMove, SortableItem, SortableList,
};

// ── Guided tour / spotlight onboarding ─────────────────────────────────────
pub use crate::{Tour, TourPlacement, TourStep};

// ── Learning surfaces (ui-learn) ───────────────────────────────────────────
// Course structure & progress, quizzes, flashcards (SM-2), gamification, and
// certificates. Pure helpers (`grade_answer`, `next_review`, `course_progress`)
// are re-exported for frontend previews; the backend re-implements grading and
// scheduling authoritatively.
pub use kinetics::prelude::{
    course_progress, grade_answer, next_review, normalize_short_answer, AchievementUnlock,
    CertificateCard, CourseLesson, CourseModule, CourseOutline, CourseProgressCard, Flashcard,
    FlashcardDeck, FlipCard, Leaderboard, LeaderboardEntry, LessonState, QuestionCard, QuizAnswer,
    QuizChoice, QuizPrompt, QuizQuestion, QuizResults, QuizTimer, ResumeLearning, ReviewRating,
    ReviewState, StreakBadge, XpBar,
};
