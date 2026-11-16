//! The unit tests, for `zig build test`: every file's tests, on this
//! machine instead of in WebAssembly.

test {
    _ = @import("appos.zig");
    _ = @import("calculator/domain.zig");
    _ = @import("calculator/links.zig");
    _ = @import("calculator/quote_usecase.zig");
    _ = @import("calculator/web.zig");
}
