// Make cargo rebuild this crate when a migration is added or edited.
//
// `db::run_migrations` calls `sqlx::migrate!("../../migrations")`, which is a
// PROC MACRO: it reads the directory and bakes every migration into the binary
// at COMPILE time. Cargo, however, only tracks `.rs` files as inputs. So adding
// a migration changes nothing cargo can see, it skips recompiling the crate,
// and the resulting binary still carries the OLD migration set.
//
// That failure is silent and convincing. The Docker `COPY migrations/` layer
// invalidates, the image rebuilds, the container restarts, `readyz` reports
// `database: ok` -- and the migration was never in the binary to run. Measured
// on exactly this repo: after adding
// `20260902000001_analytics_daily_activity_indexes.sql` and rebuilding, the
// backend logged no migration activity, `_sqlx_migrations` stayed at 90 rows,
// none of the four indexes existed, and `strings` on the shipped binary found
// zero occurrences of the migration name.
//
// `rerun-if-changed` on the directory fixes it: cargo re-runs this script when
// the directory's mtime changes (any file added or removed) and, because the
// script has no other inputs, that invalidates the crate and forces the macro
// to re-read the directory. The individual files are listed too, since a
// directory's mtime does not change when a file's CONTENTS are edited in place.
fn main() {
    let dir = std::path::Path::new("../../migrations");
    println!("cargo:rerun-if-changed={}", dir.display());
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }
}
