//! The calculator's handler: parse the query, get the prices, and turn
//! them into the page or, while the visitor types and clicks, into the
//! parts that change.

const std = @import("std");
const appos = @import("../appos.zig");
const domain = @import("domain.zig");
const links = @import("links.zig");
const quoteU = @import("quote_usecase.zig").quoteU;
const view = @import("view.zig");
const Billing = domain.Billing;
const Cents = domain.Cents;
const Plan = domain.Plan;
const PlanQuote = domain.PlanQuote;
const Quote = domain.Quote;
const SliderStop = domain.SliderStop;
const TeamSize = domain.TeamSize;
const TeamSizeError = domain.TeamSizeError;

/// The team size when the page opens without one: all three plans fit.
const default_users = "10";

/// Where the slider's marks go: the plans' limits.
const marks = [_]u32{ 1, 10, 100, 250 };

/// Everything the page shows, ready for the templates in view.zt.
pub const Index = struct {
    base: []const u8,
    /// What was typed, as typed.
    users: []const u8,
    /// The slider's stop, and its last one.
    slider: u32,
    slider_last: u32,
    marks: []const Mark,
    yearly: bool,
    ai: bool,
    /// Hidden fields, so the UTM parameters stay in the page's address.
    utm: []const appos.Pair,
    err: ?TeamSizeError,
    /// "Talk to sales" without a team size, for the error message.
    sales: []const u8,
    summary: []const u8,
    cards: []const Card,
    enterprise: ?Enterprise,
    /// This page's address with the choice, for sales to send.
    share: ?[]const u8,

    pub fn init(base: []const u8, typed: []const u8, users: ?TeamSize, billing: Billing, ai: bool, utm: links.Utm, quote: Quote, err: ?TeamSizeError) Index {
        var cards: std.ArrayList(Card) = .empty;
        var enterprise: ?Enterprise = null;
        var summary_text: []const u8 = undefined;
        switch (quote) {
            .enterprise => |u| {
                summary_text = appos.fmt("Prices for {s} users", .{domain.thousands(u.get())});
                enterprise = .{ .users = domain.thousands(u.get()), .sales = links.sales(u, utm) };
            },
            .plans => |plans| {
                summary_text = summary(users, billing);
                for (plans) |q| cards.append(appos.allocator, .init(q, users, ai, utm)) catch @panic("out of memory");
            },
        }
        const shown = users orelse (TeamSize.parse(default_users) catch unreachable);
        var mark_list: std.ArrayList(Mark) = .empty;
        for (marks) |n| {
            const stop = SliderStop.nearest(TeamSize.parse(appos.fmt("{d}", .{n})) catch continue);
            const at = @as(f64, @floatFromInt(stop.get())) / @as(f64, @floatFromInt(SliderStop.last));
            mark_list.append(appos.allocator, .{ .label = appos.fmt("{d}", .{n}), .at = appos.fmt("{d:.3}", .{at}) }) catch @panic("out of memory");
        }
        return .{
            .base = base,
            .users = typed,
            .slider = SliderStop.nearest(shown).get(),
            .slider_last = SliderStop.last,
            .marks = mark_list.items,
            .yearly = billing == .yearly,
            .ai = ai,
            .utm = utm.pairs,
            .err = err,
            .sales = links.sales(null, utm),
            .summary = summary_text,
            .cards = cards.items,
            .enterprise = enterprise,
            .share = if (users) |u| links.share(base, u, billing, ai, utm) else null,
        };
    }
};

/// One plan's card, ready to show.
pub const Card = struct {
    id: []const u8,
    name: []const u8,
    tagline: []const u8,
    popular: bool,
    /// False: greyed out, with `reason`.
    available: bool = true,
    reason: ?[]const u8 = null,
    price: []const u8,
    price_unit: []const u8,
    ai_note: []const u8,
    ai_included: bool,
    /// Null without a valid team size.
    total: ?[]const u8 = null,
    total_unit: []const u8,
    minimum: ?[]const u8 = null,
    saving: ?[]const u8 = null,
    /// "Start free trial"; null when the plan doesn't fit.
    signup: ?[]const u8,
    features: []const []const u8,

    fn init(q: PlanQuote, users: ?TeamSize, ai: bool, utm: links.Utm) Card {
        const plan = q.plan;
        const yearly = q.billing == .yearly;
        var card: Card = .{
            .id = plan.id(),
            .name = plan.name(),
            .tagline = plan.tagline(),
            .popular = plan.popular(),
            .price = q.per_user.price(),
            .price_unit = if (yearly) "per user/month, billed yearly" else "per user/month",
            .ai_note = switch (q.ai) {
                .included => "AI planning assistant: Included",
                .added => |c| appos.fmt("Includes AI planning assistant (+{s})", .{c.price()}),
                .available => |c| appos.fmt("AI planning assistant: +{s}", .{c.price()}),
            },
            .ai_included = q.ai == .included,
            .total_unit = if (yearly) "per year" else "per month",
            .signup = links.signup(plan, users, q.billing, ai, utm),
            .features = plan.features(),
        };
        switch (q.team) {
            .unknown => {},
            .does_not_fit => |fit| {
                card.available = false;
                card.reason = fit.reason();
                card.signup = null;
            },
            .fits => |f| {
                card.total = f.total.total();
                card.minimum = minimumNote(plan, users, f.billed_users);
                card.saving = savingNote(yearly, f.yearly_savings);
            },
        }
        return card;
    }
};

/// A label under the slider, at a team size.
pub const Mark = struct {
    label: []const u8,
    /// From 0 (the left end) to 1 (the right end).
    at: []const u8,
};

pub const Enterprise = struct {
    users: []const u8,
    sales: []const u8,
};

pub fn index(req: *const appos.Request) void {
    // The slider sends its stop as `slider` and leaves the box out; the
    // box, the billing and the AI switch send what was typed as `users`.
    const slid: ?SliderStop = if (appos.query(req, "slider")) |s| SliderStop.parse(s) else null;
    const typed = if (slid) |s| appos.fmt("{d}", .{s.users().get()}) else appos.query(req, "users") orelse default_users;
    const billing = Billing.parse(appos.query(req, "billing"));
    const ai = domain.parseAi(appos.query(req, "ai"));
    const utm = links.Utm.fromQuery(req.query);
    const parsed = TeamSize.parse(typed);
    const users = parsed catch null;
    const err: ?TeamSizeError = if (parsed) |_| null else |e| e;
    const t = Index.init(req.base, typed, users, billing, ai, utm, quoteU(users, billing, ai), err);
    if (req.htmx) {
        // While typing: only the prices and the message change; the input
        // keeps its text and focus. 422 tells htmx the input didn't parse.
        if (err != null) appos.page.setStatus(422);
        appos.page.setHtml("results", appos.render(view.Results, .{&t}));
        appos.page.setHtml("users_error", appos.render(view.UsersError, .{&t}));
        // Each control moves the other, never itself: a replaced slider
        // stops the drag, a replaced box loses what is being typed.
        if (slid != null) {
            appos.page.setHtml("users_box", appos.render(view.UsersBox, .{&t}));
        } else if (err == null) {
            appos.page.setHtml("slider_box", appos.render(view.SliderBox, .{&t}));
        }
    } else {
        // A shared or mistyped link is still a working page with a hint,
        // so it answers 200, not 422.
        appos.style(@embedFile("style.css"));
        appos.title("Kitewell pricing");
        appos.page.addHeader(
            \\<meta name="description" content="Compare Kitewell plans for your team size: Starter, Team and Business, monthly or yearly.">
        );
        appos.page.setHtml("page", appos.render(view.Page, .{&t}));
    }
}

fn summary(users: ?TeamSize, billing: Billing) []const u8 {
    const billed = switch (billing) {
        .monthly => "billed monthly",
        .yearly => "billed yearly",
    };
    const u = users orelse return appos.fmt("Prices per user, {s}", .{billed});
    if (u.get() == 1) return appos.fmt("Prices for 1 user, {s}", .{billed});
    return appos.fmt("Prices for {d} users, {s}", .{ u.get(), billed });
}

/// "Billed for 10 users, the Business minimum" when the plan bills more
/// users than the team has.
fn minimumNote(plan: Plan, users: ?TeamSize, billed_users: u32) ?[]const u8 {
    const u = users orelse return null;
    if (billed_users <= u.get()) return null;
    return appos.fmt("Billed for {d} users, the {s} minimum", .{ billed_users, plan.name() });
}

fn savingNote(yearly: bool, savings: Cents) []const u8 {
    if (yearly) return appos.fmt("You save {s} a year", .{savings.total()});
    return appos.fmt("Pay yearly and save {s} a year", .{savings.total()});
}

const testing = std.testing;
const contains = std.mem.containsAtLeast;

fn has(html: []const u8, part: []const u8) bool {
    return std.mem.indexOf(u8, html, part) != null;
}

fn page(users: []const u8, billing: Billing, ai: bool, utm: links.Utm) Index {
    const parsed = TeamSize.parse(users);
    const size = parsed catch null;
    const err: ?TeamSizeError = if (parsed) |_| null else |e| e;
    return .init("/pricing", users, size, billing, ai, utm, quoteU(size, billing, ai), err);
}

fn yearlyPage(users: []const u8) Index {
    return page(users, .yearly, false, .{});
}

fn cardOf(t: Index, id: []const u8) Card {
    for (t.cards) |c| {
        if (std.mem.eql(u8, c.id, id)) return c;
    }
    @panic("no such card");
}

test "the page opens yearly with all plans" {
    const html = appos.render(view.Page, .{&yearlyPage(default_users)});
    try testing.expect(has(html, "value=\"yearly\" checked"));
    try testing.expect(!has(html, "value=\"monthly\" checked"));
    try testing.expect(has(html, "Most popular"));
    try testing.expect(has(html, "Prices for 10 users, billed yearly"));
    try testing.expect(has(html, "Prices in US dollars, excluding VAT and sales tax."));
    try testing.expect(has(html, "hx-replace-url=\"true\""));
}

test "nora's example: 25 on team, yearly" {
    const t = yearlyPage("25");
    const team = cardOf(t, "team");
    try testing.expectEqualStrings("$24.17", team.price);
    try testing.expectEqualStrings("per user/month, billed yearly", team.price_unit);
    try testing.expectEqualStrings("$7,250", team.total.?);
    try testing.expectEqualStrings("You save $1,450 a year", team.saving.?);
    const html = appos.render(view.Results, .{&t});
    try testing.expect(has(html, "$24.17"));
    try testing.expect(has(html, "$7,250"));
    try testing.expect(has(html, "You save $1,450 a year"));
    try testing.expect(has(html, "https://app.kitewell.example/signup?plan=team&amp;users=25&amp;billing=yearly&amp;ai=no"));
}

test "plans that don't fit are greyed out, not hidden" {
    const t = yearlyPage("2");
    const team = cardOf(t, "team");
    try testing.expect(!team.available);
    try testing.expectEqualStrings("Team starts at 3 users", team.reason.?);
    try testing.expectEqual(null, team.signup);
    const html = appos.render(view.Results, .{&t});
    try testing.expect(has(html, "Team starts at 3 users"));
    try testing.expect(has(html, "Most popular"));
    try testing.expect(has(html, "is-out"));
    try testing.expectEqualStrings("Starter is for up to 10 users", cardOf(yearlyPage("40"), "starter").reason.?);
}

test "the business minimum is said plainly" {
    const business = cardOf(yearlyPage("6"), "business");
    try testing.expectEqualStrings("Billed for 10 users, the Business minimum", business.minimum.?);
    try testing.expectEqualStrings("AI planning assistant: Included", business.ai_note);
    try testing.expectEqual(null, cardOf(yearlyPage("10"), "business").minimum);
}

test "above 250 only enterprise" {
    const t = page("300", .yearly, false, .fromQuery(&.{.{ "utm_source", "google" }}));
    try testing.expectEqual(0, t.cards.len);
    const html = appos.render(view.Results, .{&t});
    try testing.expect(has(html, "Talk to sales"));
    try testing.expect(has(html, "https://kitewell.example/contact-sales?users=300&amp;utm_source=google"));
    try testing.expect(!has(html, "Start free trial"));
    try testing.expect(has(appos.render(view.Results, .{&yearlyPage("100000")}), "100,000 users"));
}

test "strange input never breaks the page" {
    for ([_][]const u8{ "0", "-5", "2.5", "twenty", "", "99999999999", "<script>" }) |input| {
        const t = yearlyPage(input);
        errdefer std.debug.print("input {s}\n", .{input});
        try testing.expect(t.err != null);
        const html = appos.render(view.Page, .{&t});
        try testing.expect(!has(html, "NaN"));
        try testing.expect(!has(html, "$0 "));
        try testing.expect(!has(html, "$0<"));
        try testing.expect(!has(html, "<script>"));
        try testing.expect(has(html, "Enter a number of users from 1 to 250."));
        try testing.expect(has(html, "$24.17")); // prices per user still show
        try testing.expect(has(html, "Enter your team size to see the total."));
        try testing.expect(has(appos.render(view.UsersError, .{&t}), "More than 250?"));
    }
}

test "the slider follows the box and the box the slider" {
    const t = yearlyPage("25");
    try testing.expectEqual(20, t.slider); // 25 users is stop 20
    try testing.expect(has(appos.render(view.SliderBox, .{&t}), "type=\"range\" name=\"slider\" form=\"kw-slider\" min=\"0\" max=\"50\" step=\"1\" value=\"20\""));
    try testing.expect(has(appos.render(view.UsersBox, .{&t}), "value=\"25\""));
    const want = [_]Mark{ .{ .label = "1", .at = "0.000" }, .{ .label = "10", .at = "0.180" }, .{ .label = "100", .at = "0.700" }, .{ .label = "250", .at = "1.000" } };
    try testing.expectEqualDeep(@as([]const Mark, &want), t.marks);
    try testing.expectEqual(50, yearlyPage("300").slider); // Enterprise sits at the end
    try testing.expectEqual(9, yearlyPage("twenty").slider); // a typo leaves it at the default, 10 users
}

test "what was typed stays in the field" {
    try testing.expect(has(appos.render(view.Page, .{&yearlyPage("twenty")}), "value=\"twenty\""));
}

test "no error renders no message" {
    try testing.expectEqualStrings("", std.mem.trim(u8, appos.render(view.UsersError, .{&yearlyPage("25")}), " \n"));
}

test "monthly with ai" {
    const t = page("25", .monthly, true, .{});
    const team = cardOf(t, "team");
    try testing.expectEqualStrings("$37", team.price);
    try testing.expectEqualStrings("per user/month", team.price_unit);
    try testing.expectEqualStrings("$925", team.total.?);
    try testing.expectEqualStrings("Includes AI planning assistant (+$8)", team.ai_note);
    try testing.expectEqualStrings("Pay yearly and save $1,850 a year", team.saving.?);
    const html = appos.render(view.Page, .{&t});
    try testing.expect(has(html, "value=\"monthly\" checked"));
    try testing.expect(has(html, "value=\"yes\" role=\"switch\" checked"));
}

test "utm parameters stay in the form and the links" {
    const t = page("25", .yearly, true, .fromQuery(&.{.{ "utm_campaign", "spring" }}));
    const html = appos.render(view.Page, .{&t});
    try testing.expect(has(html, "<input type=\"hidden\" name=\"utm_campaign\" value=\"spring\">"));
    try testing.expect(has(html, "ai=yes&amp;utm_campaign=spring"));
    try testing.expectEqualStrings("/pricing?users=25&billing=yearly&ai=yes&utm_campaign=spring", t.share.?);
}
