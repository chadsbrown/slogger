// `sqlx::migrate!` embeds the migrations at compile time; without this a
// new migration file doesn't trigger a rebuild and is silently missing.
fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
