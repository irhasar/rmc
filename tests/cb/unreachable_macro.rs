// Regression: as `todo_macro`, for `unreachable!()` and its
// `core::panicking::unreachable_default_msg` helper.
fn main() {
    unreachable!();
}
