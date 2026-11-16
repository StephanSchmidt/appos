//! Kitewell's price calculator, an AppOS app in Zig. `appos build` compiles
//! it to WebAssembly with zig and wraps it in a component; the shell runs it
//! like any other app.
//!
//! One module, calculator, with the layers every module has:
//!   index.zig           its routes and menu entries
//!   web.zig             the handler: parses the query (team size, billing,
//!                       AI assistant, UTM parameters), gets the prices and
//!                       renders the page or, for htmx, the parts that change
//!   view.zt             the page and its parts, as zt templates
//!   quote_usecase.zig   the prices for a choice
//!   links.zig           the sign-up, sales and share links
//!   domain.zig          plans, prices, team sizes and money: plain Zig
//! There is no db.zig: the calculator stores nothing about visitors.
//! Each file tests its layer at the bottom; `zig build test` runs them all.

const wit = @import("wit");
const appos = @import("appos.zig");
const calculator = @import("calculator/index.zig");

/// Every module of the app.
const modules = [_]appos.Module{calculator.module};

/// What the shell calls: `init` once, `start` per request.
pub const handler = struct {
    pub fn init() void {
        appos.register(&modules);
    }

    pub fn start(name: []const u8, req: appos.Request) void {
        appos.dispatch(&modules, name, &req);
        appos.page.done();
    }
};

pub const panic = appos.panic;

comptime {
    wit.exports(@This());
}
