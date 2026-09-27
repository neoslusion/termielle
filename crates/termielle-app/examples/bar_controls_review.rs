//! Read-only inspection of the live bar integrations.
fn main() {
    println!(
        "volume: {:?}",
        termielle_app::bar::volume::try_query_volume()
    );
    println!(
        "workspaces: {:?}",
        termielle_app::bar::workspaces::try_query_workspaces()
    );
}
