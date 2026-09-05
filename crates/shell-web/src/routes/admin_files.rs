// crates/shell-web/src/routes/admin_files.rs
//
// Admin file-assets browser. Lists the most-recent 200 available files in
// the tenant. Mutation actions (delete) are deferred to a follow-up; this
// page is a read-only inventory.
use design_system::kinetics_ui::{DataTable, DataTableColumn, DataTableRow};
use design_system::{EmptyState, PageHeader, SkeletonCard};
use dioxus::prelude::*;
use dioxus_router::use_navigator;
use features_courses::api;
use features_courses::app_shell::{AppShell, ShellUser};

use crate::route_enum::Route;
use crate::routes::{use_api, use_user_context};

fn human_size(bytes: i64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.2} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes >= 1_000_000 {
        format!("{:.2} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Format an RFC3339 timestamp as `MMM D, YYYY h:mm AM/PM`, falling back to
/// the raw string when parsing fails.
fn format_ts(raw: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(raw) {
        Ok(dt) => dt.format("%b %-d, %Y %-I:%M %p").to_string(),
        Err(_) => raw.to_string(),
    }
}

/// The file inventory as a kinetics DataTable. Pure so it's SSR-testable.
fn files_table(assets: &[api::FileAssetSummaryDto]) -> Element {
    let columns = vec![
        DataTableColumn::new("uploaded", "Uploaded"),
        DataTableColumn::new("type", "Type"),
        DataTableColumn::new("size", "Size"),
        DataTableColumn::new("linked", "Linked entity"),
        DataTableColumn::new("id", "Asset ID"),
    ];
    let rows: Vec<DataTableRow> = assets
        .iter()
        .map(|a| {
            let linked = match (&a.linked_entity_type, &a.linked_entity_id) {
                (Some(t), Some(id)) => format!("{t}/{id}"),
                _ => "(unlinked)".to_string(),
            };
            DataTableRow::new(
                a.asset_id.clone(),
                vec![
                    format_ts(&a.created_at),
                    a.content_type.clone(),
                    human_size(a.size_bytes),
                    linked,
                    a.asset_id.clone(),
                ],
            )
        })
        .collect();
    rsx! {
        DataTable { columns, rows, caption: "File assets".to_string() }
    }
}

#[component]
pub fn AdminFiles() -> Element {
    let nav = use_navigator();
    let api = use_api();
    let user_ctx = use_user_context();

    let user = match user_ctx.read().clone() {
        Some(u) => u,
        None => {
            nav.push(Route::Login {});
            return rsx! { p { "Redirecting…" } };
        }
    };

    let is_admin = user.can_manage_organization();
    if !is_admin {
        return rsx! {
            div { class: "container",
                h1 { "Forbidden" }
                p { "You don't have permission to browse files." }
            }
        };
    }

    let assets = use_resource({
        let api = api.clone();
        move || {
            let api = api.clone();
            async move { api::list_tenant_file_assets(&api).await }
        }
    });

    let snap = assets.read_unchecked();
    let content: Element = match snap.as_ref() {
        Some(Ok(resp)) if resp.assets.is_empty() => rsx! {
            EmptyState {
                title: "No files uploaded yet.".to_string(),
                description: "Files uploaded to courses, assignments, and submissions will appear here.".to_string(),
            }
        },
        Some(Ok(resp)) => {
            let n = resp.assets.len();
            rsx! {
                p { class: "muted", "Most recent {n} files in this tenant." }
                { files_table(&resp.assets) }
            }
        }
        Some(Err(e)) => rsx! { p { class: "error", "Could not load files: {e}" } },
        None => rsx! { SkeletonCard { height: "220px".to_string() } },
    };
    let body = rsx! {
        div { class: "admin-files-page",
            PageHeader {
                title: "Files".to_string(),
                kicker: "Workspace administration".to_string(),
                subtitle: "Read-only inventory of the tenant's uploaded file assets.".to_string(),
            }
            { content }
        }
    };
    drop(snap);

    let shell_user = ShellUser {
        display_name: user.display_name.clone(),
        email: user.email.clone(),
        tenant_role: user.tenant_role,
        is_platform_admin: user.is_platform_admin,
    };

    rsx! {
        AppShell {
            user: shell_user,
            on_signout: move |_| {
                #[cfg(target_arch = "wasm32")]
                {
                    use platform_bridge::PlatformBridge;
                    spawn(async move { let _ = platform_bridge::web::WebBridge.sign_out().await; });
                }
                nav.push(Route::Login {});
            },
            { body }
        }
    }
}
