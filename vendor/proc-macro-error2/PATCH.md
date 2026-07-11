# proc-macro-error2 patch

Upstream 2.0.1 triggers `E0365` on Rust 1.97+ (private `extern crate proc_macro` re-export).

Changes vs crates.io 2.0.1:

1. `extern crate proc_macro` → `pub extern crate proc_macro`
2. `pub use proc_macro` → `pub use crate::proc_macro` in `__export`

Remove this vendor directory when upstream releases a fixed version.
