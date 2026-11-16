//! The calculator's data and rules: plans, prices, team sizes and money.
//! Plain Zig, no IO.

const std = @import("std");
const appos = @import("../appos.zig");

/// The most users a plan on this page is for. Above it: Enterprise.
pub const self_serve_max: u32 = 250;

/// The largest number the calculator takes as a team size at all. Anything
/// bigger is a typo rather than a team.
pub const team_size_max: u64 = 1_000_000;

/// Paying yearly gives two months free: a year costs this many months.
pub const months_paid_per_year: u64 = 10;

/// The AI planning assistant, dollars per user per month, paying monthly.
pub const ai_price: u64 = 8;

/// A plan visitors can buy on the page.
pub const Plan = enum {
    starter,
    team,
    business,

    pub const all = [_]Plan{ .starter, .team, .business };

    pub fn name(p: Plan) []const u8 {
        return switch (p) {
            .starter => "Starter",
            .team => "Team",
            .business => "Business",
        };
    }

    /// The plan in links: `plan=team`.
    pub fn id(p: Plan) []const u8 {
        return @tagName(p);
    }

    /// Dollars per user per month, paying monthly.
    pub fn price(p: Plan) u64 {
        return switch (p) {
            .starter => 12,
            .team => 29,
            .business => 59,
        };
    }

    /// The smallest team the plan is for.
    pub fn minUsers(p: Plan) u32 {
        return switch (p) {
            .starter => 1,
            .team => 3,
            .business => 1,
        };
    }

    /// The largest team the plan is for.
    pub fn maxUsers(p: Plan) u32 {
        return switch (p) {
            .starter => 10,
            .team => 100,
            .business => self_serve_max,
        };
    }

    /// The fewest users the plan bills for: a team of 6 on Business pays
    /// for 10.
    pub fn billedMinimum(p: Plan) u32 {
        return switch (p) {
            .starter, .team => 1,
            .business => 10,
        };
    }

    /// Business includes the AI planning assistant; the others sell it as
    /// an add-on.
    pub fn includesAi(p: Plan) bool {
        return p == .business;
    }

    /// Always marked "Most popular".
    pub fn popular(p: Plan) bool {
        return p == .team;
    }

    pub fn tagline(p: Plan) []const u8 {
        return switch (p) {
            .starter => "For small teams, 1 to 10 users",
            .team => "For teams of 3 to 100 users",
            .business => "Up to 250 users, minimum of 10",
        };
    }

    /// What the plan includes: Nora's copy, as is.
    pub fn features(p: Plan) []const []const u8 {
        return switch (p) {
            .starter => &.{
                "Unlimited projects",
                "Calendar and timeline views",
                "Slack and Google Calendar integration",
                "Email support",
            },
            .team => &.{
                "Everything in Starter",
                "Workload view and capacity planning",
                "Guest access",
                "Priority support",
            },
            .business => &.{
                "Everything in Team",
                "SSO and SCIM",
                "Audit log",
                "AI planning assistant included",
                "Dedicated success manager",
            },
        };
    }

    /// Whether a team of `users` can buy the plan, and if not, why.
    pub fn fit(p: Plan, users: TeamSize) Fit {
        const n = users.get();
        if (n < p.minUsers()) return .{ .too_few = .{ .plan = p, .limit = p.minUsers() } };
        if (n > p.maxUsers()) return .{ .too_many = .{ .plan = p, .limit = p.maxUsers() } };
        return .fits;
    }
};

/// Whether a plan fits a team size.
pub const Fit = union(enum) {
    fits,
    too_few: Limit,
    too_many: Limit,

    pub const Limit = struct { plan: Plan, limit: u32 };

    /// Why the plan doesn't fit, for the visitor: "Team starts at 3 users".
    pub fn reason(f: Fit) ?[]const u8 {
        return switch (f) {
            .fits => null,
            .too_few => |l| appos.fmt("{s} starts at {d} users", .{ l.plan.name(), l.limit }),
            .too_many => |l| appos.fmt("{s} is for up to {d} users", .{ l.plan.name(), l.limit }),
        };
    }
};

/// How the visitor pays.
pub const Billing = enum {
    monthly,
    yearly,

    /// From a link or the form. Yearly unless it says monthly: it is what
    /// the page opens with, and a mangled link still shows prices.
    pub fn parse(text: ?[]const u8) Billing {
        const t = text orelse return .yearly;
        return if (std.ascii.eqlIgnoreCase(appos.trim(t), "monthly")) .monthly else .yearly;
    }

    /// The billing in links: `billing=yearly`.
    pub fn id(b: Billing) []const u8 {
        return @tagName(b);
    }
};

/// Whether the AI planning assistant is switched on. The form sends
/// `ai=yes`; a missing value means off.
pub fn parseAi(text: ?[]const u8) bool {
    const t = appos.trim(text orelse return false);
    for ([_][]const u8{ "yes", "on", "true", "1" }) |on| {
        if (std.ascii.eqlIgnoreCase(t, on)) return true;
    }
    return false;
}

/// Why text is not a team size. `message` is the first half of what the
/// visitor reads; the page adds what works.
pub const TeamSizeError = error{ Empty, NotANumber, NotWhole, TooSmall, TooLarge };

pub fn message(err: TeamSizeError) []const u8 {
    return switch (err) {
        error.Empty => "How many people are on your team?",
        error.NotANumber => "Please type just the number, in digits, like 25.",
        error.NotWhole => "Users come in whole numbers.",
        error.TooSmall => "A team has at least 1 user.",
        error.TooLarge => "That's more users than this calculator can price.",
    };
}

/// How many people are on the team: a whole number from 1 to
/// team_size_max. Parse, don't validate: make one with `TeamSize.parse`.
pub const TeamSize = struct {
    n: u32,

    /// Parses what a visitor typed: " 25 ", "+25", "1,000" and "25.0" are
    /// fine; "", "0", "-5", "2.5" and "twenty" are not.
    pub fn parse(input: []const u8) TeamSizeError!TeamSize {
        var text = appos.trim(input);
        if (text.len == 0) return error.Empty;
        var negative = false;
        if (isMinus(text)) {
            negative = true;
            while (isMinus(text)) text = text[if (text[0] == '-') 1 else minus.len..];
            text = appos.trimStart(text);
        } else if (text[0] == '+') {
            text = appos.trimStart(text[1..]);
        }
        const whole, const fraction = splitDecimal(withoutThousandsSeparators(text));
        if (!digits(whole) or (fraction != null and !digits(fraction.?))) return error.NotANumber;
        if (negative and !(zero(whole) and (fraction == null or zero(fraction.?)))) return error.TooSmall;
        if (fraction) |f| if (!zero(f)) return error.NotWhole;
        // Digits only from here; too many of them don't fit a u64.
        const users = std.fmt.parseUnsigned(u64, whole, 10) catch return error.TooLarge;
        if (users == 0) return error.TooSmall;
        if (users > team_size_max) return error.TooLarge;
        return .{ .n = @intCast(users) };
    }

    pub fn get(t: TeamSize) u32 {
        return t.n;
    }

    /// More than 250 users: only Enterprise, through sales.
    pub fn isEnterprise(t: TeamSize) bool {
        return t.n > self_serve_max;
    }
};

/// U+2212, the minus sign some keyboards and phones type.
const minus = "\u{2212}";

fn isMinus(text: []const u8) bool {
    return std.mem.startsWith(u8, text, "-") or std.mem.startsWith(u8, text, minus);
}

/// A stop on the slider under the team size. The stops are one user apart
/// up to 20, five apart up to 100 and ten apart up to 250, so a team of 7
/// is as easy to pick as a team of 170.
pub const SliderStop = struct {
    n: u32,

    /// The last stop, 250 users.
    pub const last: u32 = 50;

    /// The stop the browser sends, 0 to last. Anything else, such as a
    /// hand-edited link, is no stop: the page then goes by `users`.
    pub fn parse(input: []const u8) ?SliderStop {
        const n = parseU32(appos.trim(input)) orelse return null;
        return if (n <= last) .{ .n = n } else null;
    }

    /// The stop nearest to a team size; above 250 users, the last one. Of
    /// two as near, the lower.
    pub fn nearest(team: TeamSize) SliderStop {
        var best: u32 = 0;
        for (1..last + 1) |stop| {
            if (distance(usersAt(@intCast(stop)), team.get()) < distance(usersAt(best), team.get())) best = @intCast(stop);
        }
        return .{ .n = best };
    }

    pub fn get(s: SliderStop) u32 {
        return s.n;
    }

    pub fn users(s: SliderStop) TeamSize {
        return .{ .n = usersAt(s.n) };
    }
};

fn usersAt(stop: u32) u32 {
    return switch (stop) {
        0...19 => stop + 1,
        20...35 => 25 + (stop - 20) * 5,
        else => 110 + (stop - 36) * 10,
    };
}

fn distance(a: u32, b: u32) u32 {
    return if (a > b) a - b else b - a;
}

/// A number as Rust's `str::parse::<u32>` reads it: digits, perhaps after
/// a `+`.
fn parseU32(text: []const u8) ?u32 {
    const rest = if (std.mem.startsWith(u8, text, "+")) text[1..] else text;
    if (!digits(rest)) return null;
    return std.fmt.parseUnsigned(u32, rest, 10) catch null;
}

/// "1,000" and "12,500" lose their commas; "2,5" keeps it (a decimal
/// comma, which splitDecimal handles).
fn withoutThousandsSeparators(text: []const u8) []const u8 {
    if (std.mem.indexOfScalar(u8, text, ',') == null) return text;
    var groups = std.mem.splitScalar(u8, text, ',');
    const first = groups.first();
    if (first.len < 1 or first.len > 3 or !digits(first)) return text;
    while (groups.next()) |g| {
        if (g.len != 3 or !digits(g)) return text;
    }
    return std.mem.replaceOwned(u8, appos.allocator, text, ",", "") catch @panic("out of memory");
}

/// "2.5" and "2,5" -> "2", "5"; "25" -> "25", null.
fn splitDecimal(text: []const u8) struct { []const u8, ?[]const u8 } {
    const i = std.mem.indexOfAny(u8, text, ".,") orelse return .{ text, null };
    return .{ text[0..i], text[i + 1 ..] };
}

fn digits(s: []const u8) bool {
    if (s.len == 0) return false;
    for (s) |b| if (!std.ascii.isDigit(b)) return false;
    return true;
}

fn zero(s: []const u8) bool {
    for (s) |b| if (b != '0') return false;
    return true;
}

/// An amount of money in US cents.
pub const Cents = struct {
    n: u64,

    pub fn dollars(d: u64) Cents {
        return .{ .n = d * 100 };
    }

    /// A total: whole dollars with thousands separators, "$7,250".
    pub fn total(c: Cents) []const u8 {
        return appos.fmt("${s}", .{thousands((c.n + 50) / 100)});
    }

    /// A price per user: "$29" when it is round, "$24.17" when not.
    pub fn price(c: Cents) []const u8 {
        const d = thousands(c.n / 100);
        const cents = c.n % 100;
        return if (cents == 0) appos.fmt("${s}", .{d}) else appos.fmt("${s}.{d:0>2}", .{ d, cents });
    }

    /// What a monthly price per user comes to per month when paid yearly:
    /// 10 months spread over 12, to the nearest cent.
    pub fn perMonthPaidYearly(c: Cents) Cents {
        return .{ .n = (c.n * months_paid_per_year + 6) / 12 };
    }
};

/// 7250 -> "7,250".
pub fn thousands(n: u64) []const u8 {
    const plain = appos.fmt("{d}", .{n});
    var out: appos.Text = .{};
    for (plain, 0..) |c, i| {
        if (i > 0 and (plain.len - i) % 3 == 0) out.byte(',');
        out.byte(c);
    }
    return out.get();
}

/// The AI planning assistant on one plan.
pub const AiPrice = union(enum) {
    /// Business: no extra cost.
    included,
    /// Switched on: the price per user includes it.
    added: Cents,
    /// Switched off: what it would add per user.
    available: Cents,
};

/// What a team pays on one plan.
pub const TeamPrice = union(enum) {
    /// No valid team size yet: only the prices per user.
    unknown,
    /// The plan isn't for a team this size.
    does_not_fit: Fit,
    fits: struct {
        /// More than the team when the plan has a minimum.
        billed_users: u32,
        /// Per month when paying monthly, per year when paying yearly.
        total: Cents,
        /// What paying yearly saves per year compared to monthly.
        yearly_savings: Cents,
    },
};

/// One plan's prices for a choice.
pub const PlanQuote = struct {
    plan: Plan,
    billing: Billing,
    /// Per user per month, the AI assistant included when it is switched
    /// on; paying yearly, the yearly price spread over 12 months.
    per_user: Cents,
    ai: AiPrice,
    team: TeamPrice,

    pub fn init(plan: Plan, billing: Billing, ai: bool, users: ?TeamSize) PlanQuote {
        const addon = !plan.includesAi() and ai;
        // Dollars per user per month, paying monthly.
        const monthly = plan.price() + if (addon) ai_price else 0;
        const ai_quote: AiPrice = if (plan.includesAi()) .included else if (ai) .{ .added = perMonth(billing, ai_price) } else .{ .available = perMonth(billing, ai_price) };
        const team: TeamPrice = if (users) |u| switch (plan.fit(u)) {
            .fits => fits: {
                const billed: u64 = @max(u.get(), plan.billedMinimum());
                const months: u64 = if (billing == .monthly) 1 else months_paid_per_year;
                break :fits .{ .fits = .{
                    .billed_users = @intCast(billed),
                    .total = .dollars(billed * monthly * months),
                    .yearly_savings = .dollars(billed * monthly * (12 - months_paid_per_year)),
                } };
            },
            else => |f| .{ .does_not_fit = f },
        } else .unknown;
        return .{ .plan = plan, .billing = billing, .per_user = perMonth(billing, monthly), .ai = ai_quote, .team = team };
    }

    fn perMonth(billing: Billing, dollars: u64) Cents {
        return switch (billing) {
            .monthly => .dollars(dollars),
            .yearly => Cents.dollars(dollars).perMonthPaidYearly(),
        };
    }
};

/// What the page shows for a choice.
pub const Quote = union(enum) {
    /// Up to 250 users, or no valid team size yet: the three plans.
    plans: [Plan.all.len]PlanQuote,
    /// More than 250 users: only Enterprise, through sales.
    enterprise: TeamSize,
};

const testing = std.testing;

fn users_(n: u32) TeamSize {
    return TeamSize.parse(appos.fmt("{d}", .{n})) catch unreachable;
}

test "team sizes people type" {
    for ([_]struct { []const u8, u32 }{
        .{ "25", 25 },
        .{ "  25 ", 25 },
        .{ "+25", 25 },
        .{ "025", 25 },
        .{ "25.0", 25 },
        .{ "1,000", 1000 },
        .{ "100000", 100_000 },
    }) |case| {
        try testing.expectEqual(case[1], (try TeamSize.parse(case[0])).get());
    }
}

test "strange team sizes" {
    for ([_]struct { []const u8, TeamSizeError }{
        .{ "", error.Empty },
        .{ "   ", error.Empty },
        .{ "0", error.TooSmall },
        .{ "-5", error.TooSmall },
        .{ "\u{2212} 5", error.TooSmall },
        .{ "-2.5", error.TooSmall },
        .{ "2.5", error.NotWhole },
        .{ "2,5", error.NotWhole },
        .{ "twenty", error.NotANumber },
        .{ "25 users", error.NotANumber },
        .{ "1e3", error.NotANumber },
        .{ "-", error.NotANumber },
        .{ ".5", error.NotANumber },
        .{ "5.", error.NotANumber },
        .{ "NaN", error.NotANumber },
        .{ "1000001", error.TooLarge },
        .{ "99999999999999999999999", error.TooLarge },
    }) |case| {
        testing.expectError(case[1], TeamSize.parse(case[0])) catch |e| {
            std.debug.print("input {s}\n", .{case[0]});
            return e;
        };
    }
}

test "minus zero is zero" {
    try testing.expectError(error.TooSmall, TeamSize.parse("-0"));
}

test "enterprise is above 250" {
    try testing.expect(!users_(250).isEnterprise());
    try testing.expect(users_(251).isEnterprise());
}

test "billing defaults to yearly" {
    try testing.expectEqual(.monthly, Billing.parse("monthly"));
    try testing.expectEqual(.monthly, Billing.parse(" Monthly "));
    try testing.expectEqual(.yearly, Billing.parse("yearly"));
    try testing.expectEqual(.yearly, Billing.parse("weekly"));
    try testing.expectEqual(.yearly, Billing.parse(null));
}

test "ai is off unless switched on" {
    try testing.expect(parseAi("yes"));
    try testing.expect(parseAi("on"));
    try testing.expect(!parseAi("no"));
    try testing.expect(!parseAi(""));
    try testing.expect(!parseAi(null));
}

test "money formats" {
    try testing.expectEqualStrings("$7,250", Cents.dollars(7250).total());
    try testing.expectEqualStrings("$1,234,567", Cents.dollars(1_234_567).total());
    try testing.expectEqualStrings("$29", Cents.dollars(29).total());
    try testing.expectEqualStrings("$29", Cents.dollars(29).price());
    try testing.expectEqualStrings("$24.17", (Cents{ .n = 2417 }).price());
    try testing.expectEqualStrings("$10.05", (Cents{ .n = 1005 }).price());
    try testing.expectEqual(2417, Cents.dollars(29).perMonthPaidYearly().n);
    try testing.expectEqual(1000, Cents.dollars(12).perMonthPaidYearly().n);
    try testing.expectEqual(667, Cents.dollars(8).perMonthPaidYearly().n);
    try testing.expectEqualStrings("0", thousands(0));
    try testing.expectEqualStrings("100", thousands(100));
    try testing.expectEqualStrings("1,000", thousands(1000));
}

test "team of 25 on team yearly" {
    const q = PlanQuote.init(.team, .yearly, false, users_(25));
    try testing.expectEqualStrings("$24.17", q.per_user.price());
    try testing.expectEqual(TeamPrice{ .fits = .{ .billed_users = 25, .total = .dollars(7250), .yearly_savings = .dollars(1450) } }, q.team);
    try testing.expectEqual(AiPrice{ .available = .{ .n = 667 } }, q.ai);
}

test "team of 25 on team monthly with ai" {
    const q = PlanQuote.init(.team, .monthly, true, users_(25));
    try testing.expectEqualStrings("$37", q.per_user.price());
    try testing.expectEqual(AiPrice{ .added = .dollars(8) }, q.ai);
    try testing.expectEqual(TeamPrice{ .fits = .{ .billed_users = 25, .total = .dollars(925), .yearly_savings = .dollars(1850) } }, q.team);
}

test "the free months apply to the ai assistant" {
    const q = PlanQuote.init(.starter, .yearly, true, users_(4));
    try testing.expectEqualStrings("$16.67", q.per_user.price());
    try testing.expectEqual(Cents.dollars(4 * 20 * 10), q.team.fits.total);
}

test "business includes ai and bills at least 10" {
    const q = PlanQuote.init(.business, .yearly, true, users_(6));
    try testing.expectEqual(AiPrice.included, q.ai);
    try testing.expectEqualStrings("$49.17", q.per_user.price());
    try testing.expectEqual(TeamPrice{ .fits = .{ .billed_users = 10, .total = .dollars(5900), .yearly_savings = .dollars(1180) } }, q.team);
}

test "plans that don't fit say why" {
    const q = PlanQuote.init(.starter, .yearly, false, users_(11));
    try testing.expectEqualStrings("Starter is for up to 10 users", q.team.does_not_fit.reason().?);
    try testing.expectEqualStrings("Team starts at 3 users", Plan.team.fit(users_(2)).reason().?);
    try testing.expectEqualStrings("Team is for up to 100 users", Plan.team.fit(users_(101)).reason().?);
    try testing.expectEqual(Fit.fits, Plan.business.fit(users_(1)));
    try testing.expectEqual(Fit.fits, Plan.business.fit(users_(250)));
}

test "the slider is fine for small teams and coarse for large ones" {
    const at = struct {
        fn at(stop: u32) u32 {
            return SliderStop.parse(appos.fmt("{d}", .{stop})).?.users().get();
        }
    }.at;
    try testing.expectEqual([_]u32{ 1, 10, 20, 25, 100, 110, 250 }, [_]u32{ at(0), at(9), at(19), at(20), at(35), at(36), at(50) });
    for (0..SliderStop.last + 1) |stop| {
        try testing.expectEqual(stop, SliderStop.nearest(users_(at(@intCast(stop)))).get());
    }
    const nearest = struct {
        fn nearest(n: u32) u32 {
            return SliderStop.nearest(users_(n)).users().get();
        }
    }.nearest;
    try testing.expectEqual([_]u32{ 20, 25, 35, 250 }, [_]u32{ nearest(22), nearest(23), nearest(37), nearest(300) });
    for ([_][]const u8{ "51", "-1", "2.5", "", "x", "1_0" }) |text| {
        try testing.expectEqual(null, SliderStop.parse(text));
    }
    try testing.expectEqual(5, SliderStop.parse(" +5 ").?.get());
}

test "no team size still has prices per user" {
    const q = PlanQuote.init(.team, .monthly, false, null);
    try testing.expectEqualStrings("$29", q.per_user.price());
    try testing.expectEqual(TeamPrice.unknown, q.team);
}

test "no price is ever zero" {
    for (Plan.all) |plan| {
        for ([_]Billing{ .monthly, .yearly }) |billing| {
            for ([_]bool{ false, true }) |ai| {
                for (1..self_serve_max + 1) |n| {
                    const q = PlanQuote.init(plan, billing, ai, .{ .n = @intCast(n) });
                    try testing.expect(q.per_user.n > 0);
                    if (q.team == .fits) {
                        try testing.expect(q.team.fits.total.n > 0);
                        try testing.expect(q.team.fits.yearly_savings.n > 0);
                    }
                }
            }
        }
    }
}
