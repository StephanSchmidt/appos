//! The price calculator: visitors type their team size, choose monthly or
//! yearly and the AI assistant, and see all plans side by side. It stores
//! nothing (app.toml grants no capabilities).

const appos = @import("../appos.zig");
const web = @import("web.zig");

/// The module's routes and menu entry: one page; the choice is in its query
/// string, so the address can be shared.
pub const module: appos.Module = .{
    .routes = &.{.get("/", web.index)},
    .menu = &.{.{ "Pricing", "/" }},
};
