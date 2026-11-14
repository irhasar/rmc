// Regression: the backend route leaves `core::panicking::todo_default_msg` in the
// body where `-Zno-codegen` folds it to `panic(const "not yet implemented")`.
// Both abort, so the front end maps the helper to the same runtime sink.
fn main() {
    todo!();
}
