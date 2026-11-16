//! Where the buttons go: the trial sign-up and sales, with the visitor's
//! choice and the campaign's UTM parameters, so attribution survives.

const std = @import("std");
const appos = @import("../appos.zig");
const domain = @import("domain.zig");
const Billing = domain.Billing;
const Plan = domain.Plan;
const TeamSize = domain.TeamSize;

pub const signup_url = "https://app.kitewell.example/signup";
pub const sales_url = "https://kitewell.example/contact-sales";

/// The UTM parameters the paid campaigns use, in this order.
pub const utm_keys = [_][]const u8{ "utm_source", "utm_medium", "utm_campaign", "utm_content", "utm_term" };

/// The longest UTM value passed on, in characters; longer ones are cut.
const utm_max = 200;

/// The UTM parameters the page was opened with, to pass on.
pub const Utm = struct {
    pairs: []const appos.Pair = &.{},

    /// Takes the five UTM parameters from the query, trimmed; empty ones
    /// are left out.
    pub fn fromQuery(query: []const appos.Pair) Utm {
        var pairs: std.ArrayList(appos.Pair) = .empty;
        for (utm_keys) |key| {
            const value = cut(appos.trim(appos.find(query, key) orelse continue), utm_max);
            if (value.len > 0) pairs.append(appos.allocator, .{ key, value }) catch @panic("out of memory");
        }
        return .{ .pairs = pairs.items };
    }
};

/// The first `max` characters of `s`.
fn cut(s: []const u8, max: usize) []const u8 {
    var chars: usize = 0;
    for (s, 0..) |b, i| {
        if (b & 0xC0 == 0x80) continue;
        if (chars == max) return s[0..i];
        chars += 1;
    }
    return s;
}

/// "Start free trial": `?plan=team&users=25&billing=yearly&ai=yes` and the
/// UTM parameters. Without a valid team size, no `users`. Business
/// includes the AI assistant, so it is always `ai=yes` there.
pub fn signup(plan: Plan, users: ?TeamSize, billing: Billing, ai: bool, utm: Utm) []const u8 {
    var query: std.ArrayList(appos.Pair) = .empty;
    add(&query, "plan", plan.id());
    if (users) |u| add(&query, "users", appos.fmt("{d}", .{u.get()}));
    add(&query, "billing", billing.id());
    add(&query, "ai", if (ai or plan.includesAi()) "yes" else "no");
    return withQuery(signup_url, query.items, utm);
}

/// "Talk to sales": `?users=300` and the UTM parameters.
pub fn sales(users: ?TeamSize, utm: Utm) []const u8 {
    var query: std.ArrayList(appos.Pair) = .empty;
    if (users) |u| add(&query, "users", appos.fmt("{d}", .{u.get()}));
    return withQuery(sales_url, query.items, utm);
}

/// This page with the choice filled in, for sales to send: opening it
/// shows the same prices. `base` is the app's path, "/pricing".
pub fn share(base: []const u8, users: TeamSize, billing: Billing, ai: bool, utm: Utm) []const u8 {
    var query: std.ArrayList(appos.Pair) = .empty;
    add(&query, "users", appos.fmt("{d}", .{users.get()}));
    add(&query, "billing", billing.id());
    if (ai) add(&query, "ai", "yes");
    return withQuery(base, query.items, utm);
}

fn add(query: *std.ArrayList(appos.Pair), key: []const u8, value: []const u8) void {
    query.append(appos.allocator, .{ key, value }) catch @panic("out of memory");
}

fn withQuery(url: []const u8, query: []const appos.Pair, utm: Utm) []const u8 {
    var out: appos.Text = .{};
    out.add(url);
    var i: usize = 0;
    for ([_][]const appos.Pair{ query, utm.pairs }) |pairs| {
        for (pairs) |p| {
            out.byte(if (i == 0) '?' else '&');
            appos.encodeInto(&out, p[0]);
            out.byte('=');
            appos.encodeInto(&out, p[1]);
            i += 1;
        }
    }
    return out.get();
}

const testing = std.testing;

fn users_(n: []const u8) ?TeamSize {
    return TeamSize.parse(n) catch unreachable;
}

test "signup carries the choice" {
    try testing.expectEqualStrings(
        "https://app.kitewell.example/signup?plan=team&users=25&billing=yearly&ai=yes",
        signup(.team, users_("25"), .yearly, true, .{}),
    );
    try testing.expectEqualStrings(
        "https://app.kitewell.example/signup?plan=starter&users=4&billing=monthly&ai=no",
        signup(.starter, users_("4"), .monthly, false, .{}),
    );
}

test "business always has ai" {
    try testing.expect(std.mem.endsWith(u8, signup(.business, users_("40"), .yearly, false, .{}), "plan=business&users=40&billing=yearly&ai=yes"));
}

test "signup without a team size" {
    try testing.expectEqualStrings("https://app.kitewell.example/signup?plan=team&billing=yearly&ai=no", signup(.team, null, .yearly, false, .{}));
}

test "utm parameters are passed on encoded" {
    const utm = Utm.fromQuery(&.{
        .{ "utm_campaign", "spring sale & more" },
        .{ "utm_source", "google" },
        .{ "utm_term", "" },
        .{ "other", "x" },
    });
    try testing.expectEqualStrings(
        "https://kitewell.example/contact-sales?users=300&utm_source=google&utm_campaign=spring%20sale%20%26%20more",
        sales(users_("300"), utm),
    );
    try testing.expect(std.mem.endsWith(u8, signup(.team, users_("25"), .yearly, true, utm), "ai=yes&utm_source=google&utm_campaign=spring%20sale%20%26%20more"));
}

test "long utm values are cut" {
    const long: [500]u8 = @splat('x');
    try testing.expectEqual(200, Utm.fromQuery(&.{.{ "utm_source", &long }}).pairs[0][1].len);
    var cafe: appos.Text = .{};
    for (0..300) |_| cafe.add("é");
    try testing.expectEqualStrings(cafe.get()[0 .. 2 * 200], Utm.fromQuery(&.{.{ "utm_source", cafe.get() }}).pairs[0][1]);
}

test "sales without a team size" {
    try testing.expectEqualStrings("https://kitewell.example/contact-sales", sales(null, .{}));
}

test "share link" {
    const utm = Utm.fromQuery(&.{.{ "utm_source", "sales" }});
    try testing.expectEqualStrings("/pricing?users=40&billing=yearly&utm_source=sales", share("/pricing", users_("40").?, .yearly, false, utm));
}
