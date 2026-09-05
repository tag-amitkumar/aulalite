// crates/design-system/src/lib.rs
//! Design system primitives. Real components added in Section F.

pub mod auth_hero;
pub mod avatar;
pub mod badge;
pub mod brand;
pub mod button;
pub mod card;
pub mod card_list;
pub mod checkbox;
pub mod course_cover_art;
mod csp_overlays;
pub mod datetime_picker;
pub mod dropdown_menu;
pub mod empty_state;
pub mod field;
pub mod file_card;
pub mod form_error;
pub mod i18n;
pub mod icon;
pub mod illustration;
pub mod input;
pub mod kinetics_styles;
/// Curated `dioxus-kinetics` component re-exports (glass, metrics, command
/// palette, motion, AI surfaces). Use `design_system::kinetics_ui::*`.
pub mod kinetics_ui;
pub mod loading;
pub mod markdown_editor;
pub mod modal;
pub mod native_styles;
pub mod page_header;
pub mod progress_bar;
pub mod select;
pub mod sheet;
pub mod skeleton;
pub mod spinner;
pub mod switch;
pub mod table;
pub mod tabs;
pub mod theme;
pub mod toast;
pub mod tokens;
pub mod tooltip;

pub use auth_hero::{build_auth_hero_svg, AuthHero, AuthHeroProps};
pub use avatar::{Avatar, AvatarSize};
pub use badge::{Badge, BadgeSize, BadgeTone};
pub use brand::{AulaLogo, AulaLogoProps};
pub use button::{Button, ButtonProps, ButtonSize, ButtonVariant};
pub use card::{
    Card, CardContent, CardDescription, CardFooter, CardHeader, CardProps, CardTitle, CardVariant,
};
pub use card_list::{CardList, CardListProps};
pub use checkbox::{Checkbox, Radio};
pub use course_cover_art::{build_cover_svg, CourseCoverArt, CourseCoverArtProps};
pub use csp_overlays::{CommandGroup, CommandItem, CommandMenu, Tour, TourPlacement, TourStep};
pub use datetime_picker::DateTimePicker;
pub use dropdown_menu::{
    DropdownAlign, DropdownItemTone, DropdownMenu, DropdownMenuItem, DropdownMenuLabel,
    DropdownMenuSeparator,
};
pub use empty_state::{EmptyState, EmptyStateVariant};
pub use field::{Field, FieldProps};
pub use file_card::FileCard;
pub use form_error::{FormError, FormErrorProps};
pub use i18n::*;
pub use icon::{UiIcon, UiIconProps, UiIconView};
pub use illustration::{Illustration, IllustrationKind};
pub use input::{Input, InputProps};
pub use kinetics_styles::{kinetics_css, KineticsStyles};
pub use loading::{Loading, LoadingLayout};
pub use markdown_editor::{render_markdown_safe, MarkdownEditor};
pub use modal::{Modal, ModalSize};
pub use native_styles::NativeBaseStyles;
pub use page_header::{HeadingLevel, PageHeader, PageHeaderProps, PageHeaderVariant};
pub use progress_bar::{ProgressBar, ProgressVariant};
pub use select::{Select, SelectOption};
pub use sheet::{
    Sheet, SheetBody, SheetClose, SheetDescription, SheetFooter, SheetHeader, SheetSide, SheetTitle,
};
pub use skeleton::{SkeletonCard, SkeletonCircle, SkeletonLine, SkeletonTableRow};
pub use spinner::{Spinner, SpinnerSize};
pub use switch::{Switch, SwitchProps};
// Toggle is the deprecated name; kept as an alias to Switch for backward compat.
pub use switch::Switch as Toggle;
pub use table::{SortDir, Table, TableHeaderCell, TableHeaderCellProps, TableProps};
pub use tabs::{Tab, Tabs, TabsVariant};
pub use theme::{apply_density_preference, apply_theme_preference, ThemePreference, ThemeToggle};
pub use toast::{
    use_toast_sender, ToastEntry, ToastLevel, ToastPosition, ToastProvider, ToastQueue,
    ToastSender, ToastViewport,
};
pub use tooltip::{Tooltip, TooltipSide};
