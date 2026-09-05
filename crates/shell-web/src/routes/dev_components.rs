//! Kitchen-sink page for visual QA of the design system. Debug builds only.

#[cfg(debug_assertions)]
mod inner {
    use design_system::{
        use_toast_sender, Badge, BadgeSize, BadgeTone, Button, ButtonSize, ButtonVariant, Card,
        CardContent, CardDescription, CardFooter, CardHeader, CardTitle, CardVariant,
        DropdownAlign, DropdownItemTone, DropdownMenu, DropdownMenuItem, DropdownMenuLabel,
        DropdownMenuSeparator, Field, Input, Sheet, SheetBody, SheetClose, SheetFooter,
        SheetHeader, SheetSide, SheetTitle, SkeletonCard, SkeletonCircle, SkeletonLine,
        SkeletonTableRow, SortDir, Tab, Table, TableHeaderCell, Tabs, TabsVariant, ToastLevel,
    };
    use dioxus::prelude::*;

    #[component]
    pub fn DevComponents() -> Element {
        let mut active_tab = use_signal(|| "primary".to_string());
        let dropdown_open = use_signal(|| false);
        let sheet_open = use_signal(|| false);
        let mut input_val = use_signal(String::new);

        rsx! {
            div { class: "ds-page-enter", style: "max-width: 960px; margin: 32px auto; padding: 0 24px; display: grid; gap: 32px;",

                // --- Header section
                section { "data-stagger": "1",
                    h1 { style: "font-family: var(--font-display); font-size: 36px; margin: 0 0 8px;", "Design System Kitchen Sink" }
                    p { style: "color: var(--color-text-muted); margin: 0;", "All primitives, all variants, all states. Debug build only." }
                }

                // --- Buttons
                section { "data-stagger": "2",
                    h2 { "Buttons" }
                    div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                        Button { label: "Primary".to_string(),     on_click: |_| {} }
                        Button { label: "Secondary".to_string(),   variant: ButtonVariant::Secondary,   on_click: |_| {} }
                        Button { label: "Ghost".to_string(),       variant: ButtonVariant::Ghost,       on_click: |_| {} }
                        Button { label: "Destructive".to_string(), variant: ButtonVariant::Destructive, on_click: |_| {} }
                        Button { label: "Premium".to_string(),     variant: ButtonVariant::Premium,     on_click: |_| {} }
                        Button { label: "Link".to_string(),        variant: ButtonVariant::Link,        on_click: |_| {} }
                    }
                    div { style: "display: flex; gap: 8px; margin-top: 12px; align-items: center;",
                        Button { label: "Small".to_string(),  size: ButtonSize::Sm, on_click: |_| {} }
                        Button { label: "Medium".to_string(), size: ButtonSize::Md, on_click: |_| {} }
                        Button { label: "Large".to_string(),  size: ButtonSize::Lg, on_click: |_| {} }
                        Button { label: "Loading".to_string(), loading: true, on_click: |_| {} }
                        Button { label: "Disabled".to_string(), disabled: true, on_click: |_| {} }
                    }
                }

                // --- Cards
                section { "data-stagger": "3",
                    h2 { "Cards" }
                    div { style: "display: grid; grid-template-columns: repeat(3, 1fr); gap: 16px;",
                        Card {
                            CardHeader { CardTitle { "Default card" } CardDescription { "Flat, on the warm-paper surface." } }
                            CardContent { p { "Body content goes here." } }
                            CardFooter { Button { label: "OK".to_string(), on_click: |_| {} } }
                        }
                        Card { interactive: true,
                            CardHeader { CardTitle { "Interactive card" } CardDescription { "Hover for the warm halo." } }
                            CardContent { p { "Used for clickable surfaces." } }
                        }
                        Card { variant: CardVariant::Premium,
                            CardHeader { CardTitle { "Premium card" } CardDescription { "1px gold inner border." } }
                            CardContent { p { "Reserved for paywalled surfaces." } }
                        }
                    }
                }

                // --- Inputs / Fields
                section { "data-stagger": "4",
                    h2 { "Inputs & fields" }
                    div { style: "display: grid; grid-template-columns: 1fr 1fr; gap: 16px;",
                        Field {
                            label: "Email".to_string(),
                            helper: Some("We never share your email".to_string()),
                            Input { value: input_val.read().clone(), placeholder: "you@example.com".to_string(), on_input: move |v| input_val.set(v) }
                        }
                        Field { label: "Search".to_string(),
                            Input {
                                value: "".to_string(),
                                placeholder: "Find a course".to_string(),
                                leading_icon: Some(rsx! { span { "🔍" } }),
                                on_input: |_| {},
                            }
                        }
                        Field { label: "Price".to_string(),
                            Input {
                                value: "".to_string(),
                                addon_left: Some("$".to_string()),
                                addon_right: Some("USD".to_string()),
                                on_input: |_| {},
                            }
                        }
                        Field {
                            label: "Invalid".to_string(),
                            error: Some("Required".to_string()),
                            Input { value: "".to_string(), error: true, on_input: |_| {} }
                        }
                    }
                }

                // --- Badges
                section { "data-stagger": "5",
                    h2 { "Badges" }
                    div { style: "display: flex; gap: 6px; flex-wrap: wrap; align-items: center;",
                        Badge { label: "Neutral".to_string() }
                        Badge { label: "Primary".to_string(), tone: BadgeTone::Primary }
                        Badge { label: "Info".to_string(),    tone: BadgeTone::Info }
                        Badge { label: "Success".to_string(), tone: BadgeTone::Success }
                        Badge { label: "Warning".to_string(), tone: BadgeTone::Warning }
                        Badge { label: "Danger".to_string(),  tone: BadgeTone::Danger }
                        Badge { label: "LIVE".to_string(),    tone: BadgeTone::Live }
                        Badge { label: "Pro".to_string(),     tone: BadgeTone::Premium }
                        Badge { label: "small".to_string(),   size: BadgeSize::Sm }
                    }
                }

                // --- Table
                section { "data-stagger": "6",
                    h2 { "Table" }
                    Table {
                        striped: true,
                        sticky_header: true,
                        toolbar: Some(rsx! {
                            div { style: "display: flex; gap: 8px; flex: 1;",
                                Input { value: "".to_string(), placeholder: "Search".to_string(), on_input: |_| {} }
                                Button { label: "Filter".to_string(), variant: ButtonVariant::Secondary, on_click: |_| {} }
                            }
                        }),
                        head: rsx! {
                            tr {
                                TableHeaderCell { sortable: true, sort_dir: Some(SortDir::Asc), on_sort: Some(EventHandler::new(|_| {})), "Name" }
                                TableHeaderCell { "Email" }
                                TableHeaderCell { "Role" }
                            }
                        },
                        body: rsx! {
                            tr { td { "Ada Lovelace" } td { "ada@example.com" } td { Badge { label: "Owner".to_string(), tone: BadgeTone::Primary } } }
                            tr { td { "Alan Turing" }  td { "alan@example.com" } td { Badge { label: "Editor".to_string() } } }
                            tr { td { "Grace Hopper" } td { "grace@example.com" } td { Badge { label: "Viewer".to_string() } } }
                        },
                    }
                }

                // --- Tabs
                section { "data-stagger": "7",
                    h2 { "Tabs" }
                    Tabs {
                        tabs: vec![
                            Tab { key: "primary".to_string(),   label: "Primary".to_string(),   disabled: false },
                            Tab { key: "secondary".to_string(), label: "Secondary".to_string(), disabled: false },
                            Tab { key: "tertiary".to_string(),  label: "Tertiary".to_string(),  disabled: false },
                        ],
                        active: active_tab.read().clone(),
                        on_change: move |k| active_tab.set(k),
                    }
                    div { style: "margin-top: 16px;",
                        Tabs {
                            variant: TabsVariant::Pill,
                            tabs: vec![
                                Tab { key: "primary".to_string(),   label: "Primary".to_string(),   disabled: false },
                                Tab { key: "secondary".to_string(), label: "Secondary".to_string(), disabled: false },
                            ],
                            active: active_tab.read().clone(),
                            on_change: move |k| active_tab.set(k),
                        }
                    }
                }

                // --- Dropdown menu
                section { "data-stagger": "8",
                    h2 { "Dropdown menu" }
                    {
                        let mut open = dropdown_open;
                        rsx! {
                            DropdownMenu {
                                open,
                                align: DropdownAlign::Start,
                                trigger: rsx! {
                                    Button {
                                        label: "Open menu \u{25be}".to_string(),
                                        variant: ButtonVariant::Secondary,
                                        on_click: move |_| { let v = *open.read(); open.set(!v); },
                                    }
                                },
                                DropdownMenuLabel { "Account" }
                                DropdownMenuItem { label: "Profile".to_string(),  on_select: |_| {} }
                                DropdownMenuItem { label: "Settings".to_string(), on_select: |_| {} }
                                DropdownMenuSeparator {}
                                DropdownMenuItem { label: "Sign out".to_string(), tone: DropdownItemTone::Danger, on_select: |_| {} }
                            }
                        }
                    }
                }

                // --- Sheet
                section { "data-stagger": "9",
                    h2 { "Sheet" }
                    {
                        let mut open = sheet_open;
                        rsx! {
                            Button { label: "Open sheet".to_string(), on_click: move |_| { open.set(true); } }
                            Sheet {
                                open,
                                side: SheetSide::Right,
                                SheetClose { open }
                                SheetHeader { SheetTitle { "Edit profile" } }
                                SheetBody {
                                    p { "Body content with form fields would live here." }
                                }
                                SheetFooter {
                                    Button { label: "Cancel".to_string(), variant: ButtonVariant::Secondary, on_click: move |_| { open.set(false); } }
                                    Button { label: "Save".to_string(),   on_click: move |_| { open.set(false); } }
                                }
                            }
                        }
                    }
                }

                // --- Skeletons
                section { "data-stagger": "10",
                    h2 { "Skeletons" }
                    div { style: "display: flex; gap: 16px; align-items: center;",
                        SkeletonCircle { size: "40px".to_string() }
                        div { style: "flex: 1; display: grid; gap: 8px;",
                            SkeletonLine { width: "60%".to_string() }
                            SkeletonLine { width: "40%".to_string() }
                        }
                        SkeletonCard {}
                    }
                    table { style: "width: 100%; margin-top: 16px;",
                        SkeletonTableRow { cells: 3 }
                        SkeletonTableRow { cells: 3 }
                    }
                }

                // --- Toast
                section { "data-stagger": "11",
                    h2 { "Toast" }
                    {
                        let mut toast = use_toast_sender();
                        rsx! {
                            div { style: "display: flex; gap: 8px; flex-wrap: wrap;",
                                Button {
                                    label: "Fire info".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| { toast.push(ToastLevel::Info, "Info", "Heads up — informational toast.".to_string()); },
                                }
                                Button {
                                    label: "Fire success".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| { toast.push(ToastLevel::Success, "Success", "Saved successfully.".to_string()); },
                                }
                                Button {
                                    label: "Fire warning".to_string(),
                                    variant: ButtonVariant::Secondary,
                                    on_click: move |_| { toast.push(ToastLevel::Warning, "Warning", "Look both ways before crossing.".to_string()); },
                                }
                                Button {
                                    label: "Fire danger".to_string(),
                                    variant: ButtonVariant::Destructive,
                                    on_click: move |_| { toast.push(ToastLevel::Danger, "Danger", "Something went wrong.".to_string()); },
                                }
                                Button {
                                    label: "Fire premium".to_string(),
                                    variant: ButtonVariant::Premium,
                                    on_click: move |_| { toast.push(ToastLevel::Premium, "Premium unlocked", "Welcome to the premium tier.".to_string()); },
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(debug_assertions)]
pub use inner::DevComponents;

#[cfg(not(debug_assertions))]
mod release_shim {
    use dioxus::prelude::*;
    #[component]
    pub fn DevComponents() -> Element {
        rsx! {}
    }
}

#[cfg(not(debug_assertions))]
pub use release_shim::DevComponents;
