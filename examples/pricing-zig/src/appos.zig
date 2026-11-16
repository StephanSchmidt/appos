//! The AppOS SDK for Zig: modules and routes, the request, the page's head,
//! links, text and memory. The shell's calls themselves are in `wit`
//! (bindings/wit.zig), which `appos build` writes from wit/:
//! `appos.page.setHtml(...)`, `appos.kv.get(...)`.

const std = @import("std");
const builtin = @import("builtin");
const wit = @import("wit");

pub const Request = wit.types.Request;
pub const kv = wit.kv;
pub const log = wit.log;
pub const page = wit.page;

/// A key and its value, as the request's params, query and form have them.
pub const Pair = @Tuple(&.{ []const u8, []const u8 });

// Modules and routes

/// A route: method, path and the function that handles it. `path` is below
/// the app's base; `:name` segments arrive as `param(req, "name")`.
pub const Route = struct {
    method: []const u8,
    path: []const u8,
    handler: *const fn (req: *const Request) void,

    pub fn get(path: []const u8, handler: *const fn (req: *const Request) void) Route {
        return .{ .method = "GET", .path = path, .handler = handler };
    }

    pub fn post(path: []const u8, handler: *const fn (req: *const Request) void) Route {
        return .{ .method = "POST", .path = path, .handler = handler };
    }

    /// The name the shell hands to `start`, "POST /greet": unique as long
    /// as method and path are.
    fn name(r: Route) []const u8 {
        return fmt("{s} {s}", .{ r.method, r.path });
    }

    fn matches(r: Route, handler: []const u8) bool {
        return handler.len == r.method.len + 1 + r.path.len and
            std.mem.startsWith(u8, handler, r.method) and
            handler[r.method.len] == ' ' and
            std.mem.endsWith(u8, handler, r.path);
    }
};

/// A part of the app: its routes and its menu entries (label, path). The
/// app lists its modules once, for both `register` and `dispatch`.
pub const Module = struct {
    routes: []const Route,
    menu: []const Pair,
};

/// Registers every module's routes and menu entries, during `init`.
pub fn register(modules: []const Module) void {
    for (modules) |m| {
        for (m.routes) |r| wit.registry.addRoute(r.method, r.path, r.name());
        for (m.menu) |entry| wit.registry.addMenu(entry[0], entry[1]);
    }
}

/// Calls the route the shell named, during `start`; 404 if there is none.
pub fn dispatch(modules: []const Module, handler: []const u8, req: *const Request) void {
    for (modules) |m| {
        for (m.routes) |r| {
            if (r.matches(handler)) return r.handler(req);
        }
    }
    notFound();
}

pub fn notFound() void {
    page.setStatus(404);
    title("Not found");
    page.setHtml("page", "<h1 style=\"font-family: system-ui; text-align: center; margin: 4rem 1rem\">Not found</h1>");
}

// The request

/// A route parameter, `:event`.
pub fn param(req: *const Request, key: []const u8) ?[]const u8 {
    return find(req.params, key);
}

/// A query parameter, the first one if it is there more than once.
pub fn query(req: *const Request, key: []const u8) ?[]const u8 {
    return find(req.query, key);
}

/// A form field of a POST.
pub fn form(req: *const Request, key: []const u8) ?[]const u8 {
    return find(req.form, key);
}

pub fn find(pairs: []const Pair, key: []const u8) ?[]const u8 {
    for (pairs) |p| {
        if (std.mem.eql(u8, p[0], key)) return p[1];
    }
    return null;
}

// The page

/// Adds a stylesheet to the page's head: `appos.style(@embedFile("style.css"))`.
pub fn style(css: []const u8) void {
    page.addHeader(fmt("<style>{s}</style>", .{css}));
}

/// Sets the page's <title>.
pub fn title(text: []const u8) void {
    page.addHeader(fmt("<title>{s}</title>", .{escape(text)}));
}

/// HTML-escapes text, for addHeader; templates escape on their own.
pub fn escape(text: []const u8) []const u8 {
    var out: Text = .{};
    for (text) |c| switch (c) {
        '&' => out.add("&amp;"),
        '<' => out.add("&lt;"),
        '>' => out.add("&gt;"),
        '"' => out.add("&quot;"),
        '\'' => out.add("&#39;"),
        else => out.byte(c),
    };
    return out.get();
}

/// Renders a zt template to text: `appos.render(view.Page, .{&index})`.
pub fn render(comptime Template: type, args: Template.Args) []const u8 {
    var out: std.Io.Writer.Allocating = .init(allocator);
    Template.render(args, &out.writer) catch outOfMemory();
    return out.written();
}

// Links

/// Percent-encodes a query key or value; letters, digits and `-._~` stay.
pub fn encode(value: []const u8) []const u8 {
    var out: Text = .{};
    encodeInto(&out, value);
    return out.get();
}

pub fn encodeInto(out: *Text, value: []const u8) void {
    const hex = "0123456789ABCDEF";
    for (value) |b| {
        if (std.ascii.isAlphanumeric(b) or b == '-' or b == '.' or b == '_' or b == '~') {
            out.byte(b);
        } else {
            out.byte('%');
            out.byte(hex[b >> 4]);
            out.byte(hex[b & 15]);
        }
    }
}

// Text

/// `s` without white space at either end, Unicode's like Rust's `trim`: a
/// no-break space typed into a form goes too.
pub fn trim(s: []const u8) []const u8 {
    return trimEnd(trimStart(s));
}

pub fn trimStart(s: []const u8) []const u8 {
    var i: usize = 0;
    while (i < s.len) {
        const n = std.unicode.utf8ByteSequenceLength(s[i]) catch break;
        if (i + n > s.len) break;
        const c = std.unicode.utf8Decode(s[i..][0..n]) catch break;
        if (!isSpace(c)) break;
        i += n;
    }
    return s[i..];
}

pub fn trimEnd(s: []const u8) []const u8 {
    var end = s.len;
    while (end > 0) {
        var start = end - 1;
        while (start > 0 and s[start] & 0xC0 == 0x80) start -= 1;
        const c = std.unicode.utf8Decode(s[start..end]) catch break;
        if (!isSpace(c)) break;
        end = start;
    }
    return s[0..end];
}

/// Unicode's White_Space.
fn isSpace(c: u21) bool {
    return switch (c) {
        0x09...0x0D, 0x20, 0x85, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000 => true,
        else => false,
    };
}

/// Text being put together, in the request's memory.
pub const Text = struct {
    list: std.ArrayList(u8) = .empty,

    pub fn add(t: *Text, s: []const u8) void {
        t.list.appendSlice(allocator, s) catch outOfMemory();
    }

    pub fn byte(t: *Text, c: u8) void {
        t.list.append(allocator, c) catch outOfMemory();
    }

    pub fn print(t: *Text, comptime format: []const u8, args: anytype) void {
        t.list.print(allocator, format, args) catch outOfMemory();
    }

    pub fn get(t: *const Text) []const u8 {
        return t.list.items;
    }
};

/// Formats into new text: `appos.fmt("{d} users", .{n})`.
pub fn fmt(comptime format: []const u8, args: anytype) []const u8 {
    return std.fmt.allocPrint(allocator, format, args) catch outOfMemory();
}

// Memory and panics

/// Memory for the request. Each request runs in a fresh instance of the
/// app, so nothing needs to be freed: allocate, use, forget.
pub const allocator = if (builtin.target.cpu.arch.isWasm()) wit.allocator else std.heap.smp_allocator;

fn outOfMemory() noreturn {
    @panic("out of memory");
}

/// The app's panic handler, in main.zig: `pub const panic = appos.panic;`.
/// A panic is a bug: the message goes to the log, the request stops and
/// nothing it wrote is kept.
pub const panic = std.debug.FullPanic(struct {
    fn call(msg: []const u8, _: ?usize) noreturn {
        log.warn(fmt("panic: {s}", .{msg}));
        @trap();
    }
}.call);

test "trim takes Unicode white space off both ends" {
    try std.testing.expectEqualStrings("25", trim(" \t25\u{A0}\n"));
    try std.testing.expectEqualStrings("2 5", trim("\u{3000}2 5\u{2009}"));
    try std.testing.expectEqualStrings("", trim(" \u{A0} "));
    try std.testing.expectEqualStrings("\u{2212}5", trim("\u{2212}5"));
}

test "encode keeps only unreserved bytes" {
    try std.testing.expectEqualStrings("spring%20sale%20%26%20more", encode("spring sale & more"));
    try std.testing.expectEqualStrings("caf%C3%A9-._~", encode("café-._~"));
}

test "routes match their name exactly" {
    const r = Route.get("/", undefined);
    try std.testing.expect(r.matches("GET /"));
    try std.testing.expect(!r.matches("GET /x"));
    try std.testing.expect(!r.matches("POST /"));
    try std.testing.expect(!r.matches("GET  /"));
}
